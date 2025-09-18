#[allow(warnings)]
mod bindings;

use bindings::colinrozzi::genai_types::types::{Message, MessageContent, MessageRole};
use bindings::exports::theater::simple::actor::Guest;
use bindings::exports::theater::simple::http_handlers::Guest as HttpHandlersGuest;
use bindings::exports::theater::simple::message_server_client::Guest as MessageServerClientGuest;
use bindings::exports::theater::simple::supervisor_handlers::Guest as SupervisorHandlersGuest;
use bindings::theater::simple::http_framework::{self, HandlerId, ServerId};
use bindings::theater::simple::http_types::{HttpRequest, HttpResponse, ServerConfig};
use bindings::theater::simple::message_server_host::open_channel;
use bindings::theater::simple::random::generate_uuid;
use bindings::theater::simple::runtime::log;
use bindings::theater::simple::supervisor::spawn;
use bindings::theater::simple::types::{ChannelAccept, ChannelId};
use bindings::theater::simple::websocket_types::{MessageType, WebsocketMessage};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

struct Component;

// Chat state for the front-chat actor
#[derive(Serialize, Deserialize, Clone, Debug)]
struct FrontChatState {
    server_id: ServerId,
    chat_state_id: Option<String>,
    conversation_id: String,
    active_connections: HashMap<u64, ConnectionInfo>,
    chat_state_channel: Option<String>, // Channel ID for chat-state updates
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct ConnectionInfo {
    connected_at: u64,
}

// Client message protocol
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ClientRequest {
    #[serde(rename = "send_message")]
    SendMessage { content: String },
    #[serde(rename = "get_conversation")]
    GetConversation,
    #[serde(rename = "update_settings")]
    UpdateSettings { settings: ClientSettings },
}

#[derive(Serialize, Deserialize, Debug)]
struct ClientSettings {
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    model: Option<String>,
}

// Server response protocol with enhanced tool support
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ServerResponse {
    #[serde(rename = "message_added")]
    MessageAdded { message: ChatMessage },
    #[serde(rename = "message_update")]
    MessageUpdate { message: ChatMessage },
    #[serde(rename = "conversation_state")]
    ConversationState {
        messages: Vec<ChatMessage>,
        conversation_id: String,
    },
    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        tool_use_id: String,
        status: ToolStatus,
        partial_result: Option<Value>,
    },
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "connected")]
    Connected { conversation_id: String },
}

// Rich tool use display structures
#[derive(Serialize, Deserialize, Debug, Clone)]
struct RichToolUse {
    pub id: String,
    pub name: String,
    pub input: Value, // Parsed JSON for rich display
    pub status: ToolStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RichToolResult {
    pub tool_use_id: String,
    pub content: Value, // Parsed JSON content
    pub is_error: bool,
    pub execution_time_ms: Option<u64>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum ToolStatus {
    Pending,
    Executing,
    Completed,
    Error,
}

// Enhanced message type classification
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
enum ChatMessageType {
    Text,
    ToolCall,
    ToolResult,
    Mixed, // For messages that contain multiple types
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChatMessage {
    id: String,                           // Content hash
    parent_id: Option<String>,            // Parent message hash
    role: String,                         // "user" or "assistant"
    content: Vec<MessageContent>,         // Rich content (text, tool use, etc.)
    timestamp: u64,
    finished: Option<bool>,
    
    // Enhanced classification fields
    message_type: ChatMessageType,
    display_text: String,                 // Flattened text for simple rendering
    tool_uses: Vec<RichToolUse>,         // Parsed tool uses
    tool_results: Vec<RichToolResult>,   // Parsed tool results
    has_tools: bool,                     // Whether this message contains tools
    
    // Tool relationship tracking
    tool_call_id: Option<String>,        // For tool result messages
    related_message_id: Option<String>,   // Links tool calls to results
}

// Chat-state-proxy actor message protocol
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ChatProxyRequest {
    StartChat,
    GetChatStateActorId,
    AddMessage {
        message: Message,
    },
    GetHistory,
    #[serde(rename = "get_metadata")]
    GetMetadata,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ChatProxyResponse {
    ChatStateActorId {
        actor_id: String,
    },
    Success,
    Error {
        message: String,
    },
    History {
        messages: Vec<Value>,
    },
    #[serde(rename = "metadata")]
    Metadata {
        conversation_id: String,
        store_id: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ErrorInfo {
    pub code: String,
    pub message: String,
}

// Channel message types (from chat-state channels)
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ChannelMessage {
    #[serde(rename = "head")]
    Head { head: String },
    #[serde(rename = "chat_message")]
    ChatMessage { message: ChatMessageEntry },
}

#[derive(Serialize, Deserialize, Debug)]
struct ChatMessageEntry {
    id: String,
    parent_id: Option<String>,
    entry: MessageEntryVariant,
}

#[derive(Serialize, Deserialize, Debug)]
enum MessageEntryVariant {
    #[serde(rename = "Message")]
    Message(UserMessage),
    #[serde(rename = "Completion")]
    Completion(CompletionMessage),
}

#[derive(Serialize, Deserialize, Debug)]
struct UserMessage {
    role: String,
    content: Vec<MessageContent>,
}

#[derive(Serialize, Deserialize, Debug)]
struct CompletionMessage {
    content: Vec<MessageContent>,
    id: String,
    model: String,
    role: String,
    stop_reason: String,
    usage: Option<TokenUsage>,
}

#[derive(Serialize, Deserialize, Debug)]
struct TokenUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// --- State Management ---

fn get_state(state_bytes: &Option<Vec<u8>>) -> Result<FrontChatState, String> {
    match state_bytes {
        Some(bytes) => {
            serde_json::from_slice(bytes).map_err(|e| format!("Failed to deserialize state: {}", e))
        }
        None => Err("No state available".to_string()),
    }
}

fn set_state(state: &FrontChatState) -> Vec<u8> {
    serde_json::to_vec(state).unwrap_or_default()
}

// --- Enhanced Chat Message Helper ---

// Create enhanced chat message with tool parsing
fn create_enhanced_chat_message(
    id: String,
    parent_id: Option<String>,
    role: String,
    content: Vec<MessageContent>,
    timestamp: u64,
    finished: Option<bool>,
) -> ChatMessage {
    let (display_text, tool_uses, tool_results) = extract_enhanced_content(&content);
    let has_tools = !tool_uses.is_empty() || !tool_results.is_empty();
    
    // Determine message type based on content
    let message_type = if tool_uses.len() > 0 && tool_results.len() > 0 {
        ChatMessageType::Mixed
    } else if tool_uses.len() > 0 {
        ChatMessageType::ToolCall
    } else if tool_results.len() > 0 {
        ChatMessageType::ToolResult
    } else {
        ChatMessageType::Text
    };
    
    ChatMessage {
        id,
        parent_id,
        role,
        content,
        timestamp,
        finished,
        message_type,
        display_text,
        tool_uses,
        tool_results,
        has_tools,
        tool_call_id: None, // Will be set by specialized creation functions
        related_message_id: None,
    }
}

// Create separate messages for tool calls
fn create_separated_tool_messages(
    base_id: String,
    parent_id: Option<String>,
    role: String,
    content: Vec<MessageContent>,
    timestamp: u64,
    finished: Option<bool>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let mut current_text_content = Vec::new();
    let mut message_counter = 0;
    
    for content_item in content.iter() {
        match content_item {
            MessageContent::Text(_text) => {
                current_text_content.push(content_item.clone());
            }
            MessageContent::ToolUse(tool_use) => {
                // Create text message for accumulated content if any
                if !current_text_content.is_empty() {
                    let text_msg = create_text_only_message(
                        format!("{}-text-{}", base_id, message_counter),
                        parent_id.clone(),
                        role.clone(),
                        current_text_content.clone(),
                        timestamp,
                        Some(true),
                    );
                    messages.push(text_msg);
                    current_text_content.clear();
                    message_counter += 1;
                }
                
                // Create tool call message
                let tool_msg = create_tool_call_message(
                    format!("{}-tool-{}", base_id, message_counter),
                    if messages.is_empty() { parent_id.clone() } else { Some(messages.last().unwrap().id.clone()) },
                    role.clone(),
                    tool_use.clone(),
                    timestamp,
                    Some(false),
                );
                messages.push(tool_msg);
                message_counter += 1;
            }
            MessageContent::ToolResult(tool_result) => {
                // Find corresponding tool call
                let tool_call_msg_id = messages.iter()
                    .rev() // Search backwards
                    .find(|msg| msg.message_type == ChatMessageType::ToolCall && 
                              msg.tool_uses.iter().any(|tu| tu.id == tool_result.tool_use_id))
                    .map(|msg| msg.id.clone());
                
                let result_msg = create_tool_result_message(
                    format!("{}-result-{}", base_id, message_counter),
                    tool_call_msg_id.clone(),
                    role.clone(),
                    tool_result.clone(),
                    timestamp,
                    Some(true),
                );
                messages.push(result_msg);
                message_counter += 1;
            }
        }
    }
    
    // Handle remaining text content
    if !current_text_content.is_empty() {
        let final_text_msg = create_text_only_message(
            format!("{}-text-final", base_id),
            if messages.is_empty() { parent_id.clone() } else { Some(messages.last().unwrap().id.clone()) },
            role.clone(),
            current_text_content,
            timestamp,
            finished,
        );
        messages.push(final_text_msg);
    }
    
    // If no messages created, make a default one
    if messages.is_empty() {
        let default_msg = create_text_only_message(
            base_id,
            parent_id.clone(),
            role.clone(),
            vec![MessageContent::Text("[No content]".to_string())],
            timestamp,
            finished,
        );
        messages.push(default_msg);
    }
    
    messages
}

fn create_text_only_message(
    id: String,
    parent_id: Option<String>,
    role: String,
    content: Vec<MessageContent>,
    timestamp: u64,
    finished: Option<bool>,
) -> ChatMessage {
    let (display_text, tool_uses, tool_results) = extract_enhanced_content(&content);
    
    ChatMessage {
        id,
        parent_id,
        role,
        content,
        timestamp,
        finished,
        message_type: ChatMessageType::Text,
        display_text,
        tool_uses,
        tool_results,
        has_tools: false,
        tool_call_id: None,
        related_message_id: None,
    }
}

fn create_tool_call_message(
    id: String,
    parent_id: Option<String>,
    role: String,
    tool_use: bindings::colinrozzi::genai_types::types::ToolUse,
    timestamp: u64,
    finished: Option<bool>,
) -> ChatMessage {
    let content = vec![MessageContent::ToolUse(tool_use.clone())];
    let (display_text, tool_uses, tool_results) = extract_enhanced_content(&content);
    
    ChatMessage {
        id,
        parent_id,
        role,
        content,
        timestamp,
        finished,
        message_type: ChatMessageType::ToolCall,
        display_text,
        tool_uses,
        tool_results,
        has_tools: true,
        tool_call_id: Some(tool_use.id.clone()),
        related_message_id: None,
    }
}

fn create_tool_result_message(
    id: String,
    parent_id: Option<String>,
    role: String,
    tool_result: bindings::colinrozzi::genai_types::types::ToolResult,
    timestamp: u64,
    finished: Option<bool>,
) -> ChatMessage {
    let content = vec![MessageContent::ToolResult(tool_result.clone())];
    let (display_text, tool_uses, tool_results) = extract_enhanced_content(&content);
    
    ChatMessage {
        id,
        parent_id: parent_id.clone(),
        role,
        content,
        timestamp,
        finished,
        message_type: ChatMessageType::ToolResult,
        display_text,
        tool_uses,
        tool_results,
        has_tools: true,
        tool_call_id: Some(tool_result.tool_use_id.clone()),
        related_message_id: parent_id,
    }
}

// --- Helper Functions ---

fn create_websocket_message(response: &ServerResponse) -> Result<WebsocketMessage, String> {
    let json_text = serde_json::to_string(response)
        .map_err(|e| format!("Failed to serialize response: {}", e))?;

    Ok(WebsocketMessage {
        ty: bindings::theater::simple::websocket_types::MessageType::Text,
        text: Some(json_text),
        data: None,
    })
}

fn broadcast_to_connections(
    state: &FrontChatState,
    response: &ServerResponse,
) -> Result<Vec<WebsocketMessage>, String> {
    if state.active_connections.is_empty() {
        return Ok(vec![]);
    }

    let ws_message = create_websocket_message(response)?;

    // For now, we'll return one message per connection
    // The WebSocket framework will handle the actual broadcasting
    Ok(vec![ws_message])
}

// --- Actor Implementation ---

impl Guest for Component {
    fn init(data: Option<Vec<u8>>, params: (String,)) -> Result<(Option<Vec<u8>>,), String> {
        let (actor_id,) = params;
        log(&format!("front-chat actor {} initializing", actor_id));

        // Generate conversation ID
        let conversation_id =
            generate_uuid().map_err(|e| format!("Failed to generate conversation ID: {}", e))?;

        // Create HTTP server configuration
        let config = ServerConfig {
            port: Some(8080),
            host: Some("0.0.0.0".to_string()),
            tls_config: None,
        };

        // Create the HTTP server
        let server_id = http_framework::create_server(&config).map_err(|e| e.to_string())?;

        // Register handlers
        let static_handler =
            http_framework::register_handler("static").map_err(|e| e.to_string())?;
        let ws_handler =
            http_framework::register_handler("websocket").map_err(|e| e.to_string())?;

        // Set up HTTP routes
        http_framework::add_route(server_id, "/", "GET", static_handler)
            .map_err(|e| e.to_string())?;
        http_framework::add_route(server_id, "/health", "GET", static_handler)
            .map_err(|e| e.to_string())?;

        // Enable WebSocket
        log("Configuring WebSocket endpoint at /ws");
        http_framework::enable_websocket(
            server_id, 
            "/ws", 
            Some(ws_handler), // connect handler - CHANGED from None to Some
            ws_handler, // message handler (required)
            Some(ws_handler), // disconnect handler - CHANGED from None to Some
        )
        .map_err(|e| format!("Failed to enable WebSocket: {}", e))?;
        
        log("WebSocket endpoint configured successfully");
log("WebSocket handlers - connect: Some, message: registered, disconnect: Some");

        // Parse configuration from init data
        let actor_config = match &data {
            Some(bytes) => {
                log("Parsing actor configuration from init data");
                match serde_json::from_slice::<serde_json::Value>(bytes) {
                    Ok(config) => {
                        log(&format!("Parsed config: {}", config));
                        config
                    }
                    Err(e) => {
                        return Err(format!("Failed to parse init configuration: {}", e));
                    }
                }
            }
            None => {
                return Err("No configuration provided - actor config is required".to_string());
            }
        };

        // Extract actor manifest path and initial state
        let manifest_path = actor_config
            .get("actor")
            .and_then(|a| a.get("manifest_path"))
            .and_then(|p| p.as_str())
            .ok_or("Missing actor.manifest_path in configuration")?;

        let initial_state = actor_config
            .get("actor")
            .and_then(|a| a.get("initial_state"))
            .cloned();

        log(&format!("Spawning actor from manifest: {}", manifest_path));

        // Prepare init bytes for the spawned actor
        let init_bytes = match initial_state {
            Some(state) => {
                log("Using provided initial state for spawned actor");
                serde_json::to_vec(&state)
                    .map_err(|e| format!("Failed to serialize initial state: {}", e))?
            }
            None => {
                log("No initial state provided for spawned actor");
                vec![]
            }
        };

        let (chat_state_id, chat_state_channel) = match spawn(
            manifest_path,
            if init_bytes.is_empty() {
                None
            } else {
                Some(&init_bytes)
            },
        ) {
            Ok(id) => {
                log(&format!(
                    "Successfully spawned actor: {}",
                    id
                ));

                // Get the actual chat-state actor ID from the proxy
                log("Getting chat-state actor ID from proxy...");
                let get_actor_id_request = ChatProxyRequest::GetChatStateActorId;
                let request_json = serde_json::to_string(&get_actor_id_request)
                    .map_err(|e| format!("Failed to serialize get actor ID request: {}", e))?;

                let channel_id = match bindings::theater::simple::message_server_host::request(&id, request_json.as_bytes()) {
                    Ok(response_bytes) => {
                        match String::from_utf8(response_bytes) {
                            Ok(response_str) => {
                                log(&format!("Get actor ID response: {}", response_str));
                                match serde_json::from_str::<ChatProxyResponse>(&response_str) {
                                    Ok(ChatProxyResponse::ChatStateActorId { actor_id }) => {
                                        log(&format!("Got chat-state actor ID: {}", actor_id));
                                        
                                        // Now open channel with the actual chat-state actor
                                        log("Opening channel with actual chat-state actor...");
                                        let subscribe_message = serde_json::json!({
                                            "type": "channel_subscribe",
                                            "channel": format!("conversation_{}", conversation_id)
                                        });

                                        let subscribe_bytes = serde_json::to_vec(&subscribe_message)
                                            .map_err(|e| format!("Failed to serialize subscribe message: {}", e))?;

                                        match open_channel(&actor_id, &subscribe_bytes) {
                                            Ok(channel_id) => {
                                                log(&format!(
                                                    "Successfully opened channel with chat-state: {}",
                                                    channel_id
                                                ));
                                                Some(channel_id)
                                            }
                                            Err(e) => {
                                                log(&format!(
                                                    "Failed to open channel with chat-state: {}",
                                                    e
                                                ));
                                                None
                                            }
                                        }
                                    }
                                    Ok(_) => {
                                        log("Unexpected response type from get actor ID");
                                        None
                                    }
                                    Err(e) => {
                                        log(&format!("Failed to parse get actor ID response: {}", e));
                                        None
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("Failed to decode get actor ID response: {}", e));
                                None
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!("Failed to get chat-state actor ID: {:?}", e));
                        None
                    }
                };

                (Some(id), channel_id)
            }
            Err(e) => {
                log(&format!("Failed to spawn actor: {}", e));
                // Continue without actor for now
                (None, None)
            }
        };

        // Start the server
        http_framework::start_server(server_id).map_err(|e| e.to_string())?;

        log(&format!(
            "front-chat server started on port 8080 for conversation {}",
            conversation_id
        ));
        log("Available endpoints:");
        log("  GET / - Chat interface");
        log("  GET /health - Health check");
        log("  WS /ws - WebSocket chat connection");

        // Save initial state
        let final_state = FrontChatState {
            server_id,
            chat_state_id,
            conversation_id,
            active_connections: HashMap::new(),
            chat_state_channel,
        };

        Ok((Some(set_state(&final_state)),))
    }
}

impl HttpHandlersGuest for Component {
    fn handle_request(
        state_bytes: Option<Vec<u8>>,
        params: (HandlerId, HttpRequest),
    ) -> Result<(Option<Vec<u8>>, (HttpResponse,)), String> {
        let (_handler_id, request) = params;

        log(&format!(
            "Handling {} request to {} (headers: {:?})",
            request.method, request.uri, request.headers
        ));

        let response = match (request.method.as_str(), request.uri.as_str()) {
            ("GET", "/") => generate_chat_interface(),
            ("GET", "/health") => generate_health_response(),
            _ => generate_404_response(),
        };

        Ok((state_bytes, (response,)))
    }

    fn handle_middleware(
        state: Option<Vec<u8>>,
        params: (HandlerId, HttpRequest),
    ) -> Result<
        (
            Option<Vec<u8>>,
            (bindings::theater::simple::http_types::MiddlewareResult,),
        ),
        String,
    > {
        let (_, request) = params;

        let middleware_result = bindings::theater::simple::http_types::MiddlewareResult {
            proceed: true,
            request,
        };

        Ok((state, (middleware_result,)))
    }

    fn handle_websocket_connect(
        state: Option<Vec<u8>>,
        params: (HandlerId, u64, String, Option<String>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (_handler_id, connection_id, _path, _protocol) = params;

        let mut state = get_state(&state)?;
        log(&format!(
            "WebSocket connection {} established",
            connection_id
        ));

        // Add connection to active connections
        state.active_connections.insert(
            connection_id,
            ConnectionInfo {
                connected_at: 0, // TODO: Get actual timestamp
            },
        );
        
        log(&format!(
            "Active connections count: {}", 
            state.active_connections.len()
        ));

        // Send welcome message
        let welcome_response = ServerResponse::Connected {
            conversation_id: state.conversation_id.clone(),
        };

        if let Ok(ws_message) = create_websocket_message(&welcome_response) {
            // Send welcome message to this specific connection
            if let Err(e) =
                http_framework::send_websocket_message(state.server_id, connection_id, &ws_message)
            {
                log(&format!("Failed to send welcome message: {}", e));
            }
        }

        // Automatically load conversation history for new connections
        if let Some(chat_state_id) = &state.chat_state_id {
            log(&format!(
                "Auto-loading conversation history for new connection: {}",
                connection_id
            ));

            let get_history_request = ChatProxyRequest::GetHistory;
            if let Ok(request_json) = serde_json::to_string(&get_history_request) {
                match bindings::theater::simple::message_server_host::request(
                    &chat_state_id,
                    request_json.as_bytes(),
                ) {
                    Ok(response_bytes) => {
                        if let Ok(response_str) = String::from_utf8(response_bytes) {
                            if let Ok(ChatProxyResponse::History { messages }) = serde_json::from_str::<ChatProxyResponse>(&response_str) {
                                log(&format!(
                                    "Auto-loaded {} messages for connection {}",
                                    messages.len(),
                                    connection_id
                                ));
                                
                                // Convert and send history to this specific connection
                                if let Ok(client_messages) = convert_chat_state_messages_to_client(&messages) {
                                    let history_response = ServerResponse::ConversationState {
                                        messages: client_messages,
                                        conversation_id: state.conversation_id.clone(),
                                    };
                                    
                                    if let Ok(history_ws_message) = create_websocket_message(&history_response) {
                                        if let Err(e) = http_framework::send_websocket_message(
                                            state.server_id,
                                            connection_id,
                                            &history_ws_message,
                                        ) {
                                            log(&format!(
                                                "Failed to send history to connection {}: {}",
                                                connection_id, e
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!(
                            "Failed to auto-load history for connection {}: {:?}",
                            connection_id, e
                        ));
                    }
                }
            }
        }

        Ok((Some(set_state(&state)),))
    }

    fn handle_websocket_message(
        state: Option<Vec<u8>>,
        params: (HandlerId, u64, WebsocketMessage),
    ) -> Result<(Option<Vec<u8>>, (Vec<WebsocketMessage>,)), String> {
        let (_handler_id, connection_id, message) = params;

        let mut state = get_state(&state)?;

        match message.ty {
            bindings::theater::simple::websocket_types::MessageType::Text => {
                if let Some(text) = message.text {
                    log(&format!(
                        "Received WebSocket message from {}: {}",
                        connection_id, text
                    ));

                    // Parse client request
                    match serde_json::from_str::<ClientRequest>(&text) {
                        Ok(request) => {
                            let response_messages = handle_client_request(&mut state, request)?;
                            Ok((Some(set_state(&state)), (response_messages,)))
                        }
                        Err(e) => {
                            log(&format!("Failed to parse client request: {}", e));
                            let error_response = ServerResponse::Error {
                                message: format!("Invalid request format: {}", e),
                            };
                            let error_messages = broadcast_to_connections(&state, &error_response)?;
                            Ok((Some(set_state(&state)), (error_messages,)))
                        }
                    }
                } else {
                    Ok((Some(set_state(&state)), (vec![],)))
                }
            }
            _ => {
                // Handle other message types (ping, pong, etc.)
                Ok((Some(set_state(&state)), (vec![],)))
            }
        }
    }

    fn handle_websocket_disconnect(
        state: Option<Vec<u8>>,
        params: (HandlerId, u64),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (_handler_id, connection_id) = params;

        let mut state = get_state(&state)?;

        log(&format!(
            "WebSocket connection {} disconnected",
            connection_id
        ));

        // Remove connection from active connections
        let was_connected = state.active_connections.remove(&connection_id).is_some();
        
        log(&format!(
            "Connection {} removed: {}, remaining connections: {}", 
            connection_id, was_connected, state.active_connections.len()
        ));

        Ok((Some(set_state(&state)),))
    }
}

// --- Message Conversion Functions ---

// Convert chat-state messages to client message format with full structure preserved
fn convert_chat_state_messages_to_client(messages: &[Value]) -> Result<Vec<ChatMessage>, String> {
    let mut client_messages = Vec::new();
    
    for (i, message_value) in messages.iter().enumerate() {
        log(&format!("🔄 Converting message {}: {:?}", i, message_value));
        
        // Parse the chat-state message structure
        let id = message_value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("msg_{}", i))
            .to_string();
            
        let parent_id = message_value
            .get("parent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
            
        if let Some(entry) = message_value.get("entry") {
            match entry {
                Value::Object(entry_obj) => {
                    // Check if this is a Message or Completion
                    if let Some(message_data) = entry_obj.get("Message") {
                        // This is a user message
                        if let Some(role) = message_data.get("role").and_then(|r| r.as_str()) {
                            if let Some(content_array) = message_data.get("content").and_then(|c| c.as_array()) {
                                let rich_content = parse_content_array(content_array)?;
                                let _display_text = extract_text_from_content_array(content_array);
                                
                                let enhanced_message = create_enhanced_chat_message(
                                    id,
                                    parent_id,
                                    role.to_lowercase(),
                                    rich_content,
                                    0, // TODO: Add actual timestamp
                                    Some(true),
                                );
                                client_messages.push(enhanced_message);
                            }
                        }
                    } else if let Some(completion_data) = entry_obj.get("Completion") {
                        // This is an assistant completion
                        if let Some(content_array) = completion_data.get("content").and_then(|c| c.as_array()) {
                            let rich_content = parse_content_array(content_array)?;
                            let _display_text = extract_text_from_content_array(content_array);
                            let stop_reason = completion_data
                                .get("stop_reason")
                                .and_then(|s| s.as_str())
                                .unwrap_or("EndTurn");
                            
                            let enhanced_message = create_enhanced_chat_message(
                                id,
                                parent_id,
                                "assistant".to_string(),
                                rich_content,
                                0, // TODO: Add actual timestamp
                                Some(stop_reason == "EndTurn"),
                            );
                            client_messages.push(enhanced_message);
                        }
                    }
                }
                _ => {
                    log(&format!("⚠️ Unexpected entry format for message {}", i));
                }
            }
        }
    }
    
    log(&format!("✅ Converted {} messages to client format with full structure", client_messages.len()));
    Ok(client_messages)
}

// Parse rich content array into MessageContent types
fn parse_content_array(content_array: &[Value]) -> Result<Vec<MessageContent>, String> {
    let mut parsed_content = Vec::new();
    
    for (i, content_item) in content_array.iter().enumerate() {
        if let Some(content_obj) = content_item.as_object() {
            if let Some(text_value) = content_obj.get("Text") {
                if let Some(text) = text_value.as_str() {
                    log(&format!("📝 Item {}: Text content: '{}'", i, text));
                    parsed_content.push(MessageContent::Text(text.to_string()));
                    continue;
                }
            }
            
            // Handle enhanced tool use parsing
            if let Some(tool_use_value) = content_obj.get("ToolUse") {
                if let Some(tool_use_obj) = tool_use_value.as_object() {
                    log(&format!("🔧 Item {}: Enhanced Tool Use parsing", i));
                    
                    let id = tool_use_obj.get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    
                    let name = tool_use_obj.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown_tool")
                        .to_string();
                    
                    let default_input = Value::Object(serde_json::Map::new());
                    let input_json = tool_use_obj.get("input")
                        .unwrap_or(&default_input);
                    
                    // Convert input to JsonData (Vec<u8>)
                    match serde_json::to_vec(input_json) {
                        Ok(input_bytes) => {
                            let tool_use = bindings::colinrozzi::genai_types::types::ToolUse {
                                id,
                                name,
                                input: input_bytes,
                            };
                            
                            parsed_content.push(MessageContent::ToolUse(tool_use));
                            continue;
                        }
                        Err(e) => {
                            log(&format!("⚠️ Failed to serialize tool input: {}", e));
                            parsed_content.push(MessageContent::Text("[Invalid Tool Use]".to_string()));
                            continue;
                        }
                    }
                } else {
                    log(&format!("⚠️ Tool use value is not an object"));
                    parsed_content.push(MessageContent::Text("[Tool Use]".to_string()));
                    continue;
                }
            }
            
            // Handle enhanced tool result parsing
            if let Some(tool_result_value) = content_obj.get("ToolResult") {
                if let Some(tool_result_obj) = tool_result_value.as_object() {
                    log(&format!("📊 Item {}: Enhanced Tool Result parsing", i));
                    
                    let tool_use_id = tool_result_obj.get("tool_use_id")
                        .or_else(|| tool_result_obj.get("tool-use-id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    
                    let content_json = tool_result_obj.get("content")
                        .unwrap_or(&Value::Null);
                    
                    let is_error = tool_result_obj.get("is_error")
                        .or_else(|| tool_result_obj.get("is-error"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    
                    // Convert content to JsonData (Vec<u8>)
                    match serde_json::to_vec(content_json) {
                        Ok(content_bytes) => {
                            let tool_result = bindings::colinrozzi::genai_types::types::ToolResult {
                                tool_use_id,
                                content: content_bytes,
                                is_error,
                            };
                            
                            parsed_content.push(MessageContent::ToolResult(tool_result));
                            continue;
                        }
                        Err(e) => {
                            log(&format!("⚠️ Failed to serialize tool result content: {}", e));
                            parsed_content.push(MessageContent::Text("[Invalid Tool Result]".to_string()));
                            continue;
                        }
                    }
                } else {
                    log(&format!("⚠️ Tool result value is not an object"));
                    parsed_content.push(MessageContent::Text("[Tool Result]".to_string()));
                    continue;
                }
            }
            
            log(&format!("⚠️ Unknown content type in item {}: {:?}", i, content_obj.keys().collect::<Vec<_>>()));
        }
    }
    
    Ok(parsed_content)
}

// Extract text content from content array (for display_text field)
fn extract_text_from_content_array(content_array: &[Value]) -> String {
    let extracted: Vec<String> = content_array
        .iter()
        .enumerate()
        .filter_map(|(i, content_item)| {
            if let Some(text_obj) = content_item.as_object() {
                if let Some(text_value) = text_obj.get("Text") {
                    if let Some(text) = text_value.as_str() {
                        log(&format!("📝 Item {}: Text content: '{}'", i, text));
                        return Some(text.to_string());
                    }
                }
                // Handle tool use/results
                if text_obj.contains_key("ToolUse") {
                    log(&format!("🔧 Item {}: Tool Use", i));
                    return Some("[Tool Use]".to_string());
                }
                if text_obj.contains_key("ToolResult") {
                    log(&format!("📊 Item {}: Tool Result", i));
                    return Some("[Tool Result]".to_string());
                }
            }
            None
        })
        .collect();
    
    let result = if extracted.is_empty() {
        "[No displayable content]".to_string()
    } else {
        extracted.join(" ")
    };
    log(&format!("✅ Extracted text from array: '{}'", result));
    result
}

// --- Client Request Handling ---

fn handle_client_request(
    state: &mut FrontChatState,
    request: ClientRequest,
) -> Result<Vec<WebsocketMessage>, String> {
    match request {
        ClientRequest::SendMessage { content } => {
            log(&format!("Processing send_message: {}", content));

            // Create user message for display with rich structure
            let user_message = create_enhanced_chat_message(
                generate_uuid().map_err(|e| format!("Failed to generate message ID: {}", e))?,
                None, // Will be set properly when we get the real message from chat-state
                "user".to_string(),
                vec![MessageContent::Text(content.clone())],
                0, // TODO: Get actual timestamp
                Some(true),
            );

            // Broadcast user message immediately
            let user_response = ServerResponse::MessageAdded {
                message: user_message,
            };
            let mut response_messages = broadcast_to_connections(state, &user_response)?;

            // Send to chat-state actor if available
            if let Some(chat_state_id) = &state.chat_state_id {
                log(&format!(
                    "Sending message to chat-state-proxy actor: {}",
                    chat_state_id
                ));

                // Create genai-types message format
                let genai_message = Message {
                    role: MessageRole::User,
                    content: vec![MessageContent::Text(content)],
                };

                // Send add_message request
                let add_message_request = ChatProxyRequest::AddMessage {
                    message: genai_message,
                };

                let request_json = serde_json::to_string(&add_message_request)
                    .map_err(|e| format!("Failed to serialize chat-state request: {}", e))?;

                // Send message to chat-state actor using request for proper response handling
                match bindings::theater::simple::message_server_host::request(
                    &chat_state_id,
                    request_json.as_bytes(),
                ) {
                    Ok(response_bytes) => {
                        log("Successfully sent add_message to chat-state-proxy");

                        // Parse response if needed
                        if let Ok(response_str) = String::from_utf8(response_bytes) {
                            log(&format!("Add message response: {}", response_str));
                        }

                        // Chat-state-proxy handles completion automatically after AddMessage
                        // No need for separate GenerateCompletion request
                    }
                    Err(e) => {
                        log(&format!(
                            "Failed to send add_message to chat-state-proxy: {:?}",
                            e
                        ));
                    }
                }
            } else {
                log("No chat-state-proxy actor available, creating fallback response");

                // Fallback: create a simple response with rich structure
                let fallback_text = "Chat-state-proxy actor not available. Please try again.".to_string();
                let assistant_message = create_enhanced_chat_message(
                    generate_uuid()
                        .map_err(|e| format!("Failed to generate message ID: {}", e))?,
                    None,
                    "assistant".to_string(),
                    vec![MessageContent::Text(fallback_text.clone())],
                    0,
                    Some(true),
                );

                let assistant_response = ServerResponse::MessageAdded {
                    message: assistant_message,
                };

                response_messages.extend(broadcast_to_connections(state, &assistant_response)?);
            }

            Ok(response_messages)
        }
        ClientRequest::GetConversation => {
            log("Processing get_conversation");

            // Request history from chat-state-proxy
            if let Some(chat_state_id) = &state.chat_state_id {
                log(&format!(
                    "Requesting conversation history from chat-state-proxy: {}",
                    chat_state_id
                ));

                let get_history_request = ChatProxyRequest::GetHistory;
                let request_json = serde_json::to_string(&get_history_request)
                    .map_err(|e| format!("Failed to serialize get history request: {}", e))?;

                match bindings::theater::simple::message_server_host::request(
                    &chat_state_id,
                    request_json.as_bytes(),
                ) {
                    Ok(response_bytes) => {
                        match String::from_utf8(response_bytes) {
                            Ok(response_str) => {
                                log(&format!("Get history response: {}", response_str));
                                match serde_json::from_str::<ChatProxyResponse>(&response_str) {
                                    Ok(ChatProxyResponse::History { messages }) => {
                                        log(&format!(
                                            "Got {} messages from history",
                                            messages.len()
                                        ));
                                        
                                        // Convert chat-state messages to client format
                                        let client_messages = convert_chat_state_messages_to_client(&messages)?;
                                        
                                        let response = ServerResponse::ConversationState {
                                            messages: client_messages,
                                            conversation_id: state.conversation_id.clone(),
                                        };
                                        
                                        broadcast_to_connections(state, &response)
                                    }
                                    Ok(_) => {
                                        log("Unexpected response type from get history");
                                        let response = ServerResponse::Error {
                                            message: "Failed to get conversation history".to_string(),
                                        };
                                        broadcast_to_connections(state, &response)
                                    }
                                    Err(e) => {
                                        log(&format!(
                                            "Failed to parse get history response: {}",
                                            e
                                        ));
                                        let response = ServerResponse::Error {
                                            message: "Failed to parse conversation history".to_string(),
                                        };
                                        broadcast_to_connections(state, &response)
                                    }
                                }
                            }
                            Err(e) => {
                                log(&format!("Failed to decode get history response: {}", e));
                                let response = ServerResponse::Error {
                                    message: "Failed to decode conversation history".to_string(),
                                };
                                broadcast_to_connections(state, &response)
                            }
                        }
                    }
                    Err(e) => {
                        log(&format!("Failed to get conversation history: {:?}", e));
                        let response = ServerResponse::Error {
                            message: "Failed to request conversation history".to_string(),
                        };
                        broadcast_to_connections(state, &response)
                    }
                }
            } else {
                log("No chat-state-proxy actor available for getting conversation");
                
                // Return empty conversation
                let response = ServerResponse::ConversationState {
                    messages: vec![],
                    conversation_id: state.conversation_id.clone(),
                };
                
                broadcast_to_connections(state, &response)
            }
        }
        ClientRequest::UpdateSettings {
            settings: _settings,
        } => {
            log("Processing update_settings");

            // TODO: Forward to chat-state actor
            // For now, just acknowledge
            let response = ServerResponse::Error {
                message: "Settings update not implemented yet".to_string(),
            };

            broadcast_to_connections(state, &response)
        }
    }
}

// --- Response Generation Functions ---

fn generate_chat_interface() -> HttpResponse {
    let html_content = include_str!("../chat.html");

    HttpResponse {
        status: 200,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: Some(html_content.as_bytes().to_vec()),
    }
}

fn generate_health_response() -> HttpResponse {
    let json_body = r#"{"status":"ok","service":"front-chat","message":"Chat server is running"}"#;

    HttpResponse {
        status: 200,
        headers: vec![("Content-Type".to_string(), "application/json".to_string())],
        body: Some(json_body.as_bytes().to_vec()),
    }
}

fn generate_404_response() -> HttpResponse {
    let html_content = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>404 - Not Found</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            text-align: center;
            padding: 2rem;
            background: #f8fafc;
        }
        .container {
            max-width: 500px;
            margin: 0 auto;
            background: white;
            padding: 2rem;
            border-radius: 8px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }
        h1 { color: #dc2626; }
    </style>
</head>
<body>
    <div class="container">
        <h1>404 - Not Found</h1>
        <p>The requested page could not be found.</p>
        <a href="/">← Back to Chat</a>
    </div>
</body>
</html>"#;

    HttpResponse {
        status: 404,
        headers: vec![(
            "Content-Type".to_string(),
            "text/html; charset=utf-8".to_string(),
        )],
        body: Some(html_content.as_bytes().to_vec()),
    }
}

impl MessageServerClientGuest for Component {
    fn handle_send(
        state_bytes: Option<Vec<u8>>,
        params: (Vec<u8>,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (message_bytes,) = params;

        if let Ok(message_str) = String::from_utf8(message_bytes) {
            log(&format!("Received send message: {}", message_str));

            let mut state = get_state(&state_bytes)?;

            // Parse the message from chat-state
            match serde_json::from_str::<ChatProxyResponse>(&message_str) {
                Ok(response) => match handle_chat_proxy_response(&mut state, response) {
                    Ok(_) => log("Successfully handled chat-state response"),
                    Err(e) => log(&format!("Error handling chat-state response: {}", e)),
                },
                Err(e) => {
                    log(&format!("Failed to parse chat-state response: {}", e));
                }
            }

            Ok((Some(set_state(&state)),))
        } else {
            log("Received non-UTF8 message");
            Ok((state_bytes,))
        }
    }

    fn handle_request(
        state: Option<Vec<u8>>,
        _params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (Option<Vec<u8>>,)), String> {
        // Not used for our chat implementation
        Ok((state, (None,)))
    }

    fn handle_channel_open(
        state: Option<Vec<u8>>,
        params: (String, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>, (ChannelAccept,)), String> {
        let (from_actor_id, initial_message) = params;

        log(&format!(
            "Channel open request from actor: {}",
            from_actor_id
        ));

        let state = get_state(&state)?;
        // Check if this is from our chat-state actor
        let should_accept = match &state.chat_state_id {
            Some(chat_state_id) => {
                if from_actor_id == *chat_state_id {
                    log("Accepting channel from our chat-state actor");
                    true
                } else {
                    log(&format!(
                        "Rejecting channel from unknown actor: {}",
                        from_actor_id
                    ));
                    false
                }
            }
            None => {
                log("Rejecting channel - no chat-state actor spawned");
                false
            }
        };

        if should_accept {
            // Parse initial message if available
            if !initial_message.is_empty() {
                if let Ok(msg_str) = String::from_utf8(initial_message) {
                    log(&format!("Channel initial message: {}", msg_str));
                }
            }

            // Send confirmation message
            let response_message = serde_json::json!({
                "type": "channel_accepted",
                "message": "Channel established for real-time updates"
            });

            let response_bytes = serde_json::to_vec(&response_message).unwrap_or_default();

            Ok((
                Some(set_state(&state)),
                (ChannelAccept {
                    accepted: true,
                    message: Some(response_bytes),
                },),
            ))
        } else {
            Ok((
                Some(set_state(&state)),
                (ChannelAccept {
                    accepted: false,
                    message: None,
                },),
            ))
        }
    }

    fn handle_channel_message(
        state: Option<Vec<u8>>,
        params: (ChannelId, Vec<u8>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (channel_id, message_bytes) = params;

        log(&format!("Received channel message on {}", channel_id));

        let mut state = get_state(&state)?;

        // Parse the channel message
        if let Ok(message_str) = String::from_utf8(message_bytes) {
            log(&format!("Channel message content: {}", message_str));

            // Parse as channel message
            match serde_json::from_str::<ChannelMessage>(&message_str) {
                Ok(ChannelMessage::Head { head }) => {
                    log(&format!("Received head update: {}", head));
                    // Head updates indicate new messages are available
                    // For now, we'll just log them as they don't need immediate action
                }
                Ok(ChannelMessage::ChatMessage { message }) => {
                    log("Processing chat message from channel");
                    match handle_chat_message_update(&mut state, message) {
                        Ok(_) => log("Successfully handled chat message update"),
                        Err(e) => log(&format!("Error handling chat message: {}", e)),
                    }
                }
                Err(e) => {
                    log(&format!("Failed to parse channel message: {}", e));
                    
                    // Fallback: try to parse as generic JSON for debugging
                    if let Ok(generic) = serde_json::from_str::<serde_json::Value>(&message_str) {
                        log(&format!("Raw channel message: {:?}", generic));
                    }
                }
            }
        } else {
            log("Received non-UTF8 channel message");
        }

        Ok((Some(set_state(&state)),))
    }

    fn handle_channel_close(
        state: Option<Vec<u8>>,
        _params: (ChannelId,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        // Not used for our chat implementation
        Ok((state,))
    }
}

// Handle chat message updates from channels
fn handle_chat_message_update(
    state: &mut FrontChatState,
    chat_message: ChatMessageEntry,
) -> Result<(), String> {
    log(&format!("Processing chat message with ID: {}", chat_message.id));
    
    // Check if we should separate tool messages
    let should_separate_tools = match &chat_message.entry {
        MessageEntryVariant::Completion(completion) => {
            completion.content.iter().any(|content| {
                matches!(content, MessageContent::ToolUse(_) | MessageContent::ToolResult(_))
            })
        }
        _ => false,
    };
    
    if should_separate_tools {
        log("Creating separated tool messages");
        
        let messages = match chat_message.entry {
            MessageEntryVariant::Completion(completion) => {
                create_separated_tool_messages(
                    chat_message.id,
                    chat_message.parent_id,
                    "assistant".to_string(),
                    completion.content,
                    0, // TODO: Add actual timestamp
                    Some(completion.stop_reason == "EndTurn"),
                )
            }
            _ => unreachable!(), // We already checked this above
        };
        
        // Broadcast all separated messages
        for (index, message) in messages.iter().enumerate() {
            log(&format!(
                "Broadcasting separated message {}/{}: type={:?}, id={}", 
                index + 1, messages.len(), message.message_type, message.id
            ));
            
            let server_response = ServerResponse::MessageAdded {
                message: message.clone(),
            };
            
            for connection_id in state.active_connections.keys() {
                log(&format!("Sending separated message to connection: {}", connection_id));
                if let Ok(ws_message) = create_websocket_message(&server_response) {
                    if let Err(e) = http_framework::send_websocket_message(
                        state.server_id,
                        *connection_id,
                        &ws_message,
                    ) {
                        log(&format!(
                            "Failed to send separated message to connection {}: {}",
                            connection_id, e
                        ));
                    } else {
                        log(&format!("Successfully sent separated message to connection {}", connection_id));
                    }
                }
            }
        }
        
        return Ok(());
    }
    
    // For non-tool messages, use the original single message approach
    let display_message = match chat_message.entry {
        MessageEntryVariant::Message(user_msg) => {
            log(&format!("User message: {:?}", user_msg));
            create_enhanced_chat_message(
                chat_message.id,
                chat_message.parent_id,
                user_msg.role.to_lowercase(), // Convert "User" -> "user"
                user_msg.content,
                0, // TODO: Add actual timestamp
                Some(true),
            )
        }
        MessageEntryVariant::Completion(completion) => {
            log(&format!("Assistant completion: {}", completion.model));
            create_enhanced_chat_message(
                chat_message.id,
                chat_message.parent_id,
                "assistant".to_string(),
                completion.content,
                0, // TODO: Add actual timestamp
                Some(completion.stop_reason == "EndTurn"),
            )
        }
    };
    
    // Broadcast to WebSocket clients
    let server_response = ServerResponse::MessageAdded {
        message: display_message,
    };
    
    // Send to all active connections
    log(&format!(
        "Broadcasting message to {} active connections", 
        state.active_connections.len()
    ));
    
    for connection_id in state.active_connections.keys() {
        log(&format!("Sending message to connection: {}", connection_id));
        if let Ok(ws_message) = create_websocket_message(&server_response) {
            if let Err(e) = http_framework::send_websocket_message(
                state.server_id,
                *connection_id,
                &ws_message,
            ) {
                log(&format!(
                    "Failed to send message to connection {}: {}",
                    connection_id, e
                ));
            } else {
                log(&format!("Successfully sent message to connection {}", connection_id));
            }
        }
    }
    
    Ok(())
}

// Parse JsonData (Vec<u8>) into serde_json::Value
fn parse_json_data(json_data: &[u8]) -> Result<Value, String> {
    let json_str = std::str::from_utf8(json_data)
        .map_err(|e| format!("Invalid UTF-8 in JSON data: {}", e))?;
    
    serde_json::from_str(json_str)
        .map_err(|e| format!("Invalid JSON: {}", e))
}

// Format tool result for preview text
fn format_tool_result_preview(content: &Value) -> String {
    match content {
        Value::String(s) => {
            if s.len() > 100 {
                format!("{}...", &s[..97])
            } else {
                s.clone()
            }
        }
        Value::Object(obj) => {
            if let Some(text) = obj.get("text").and_then(|v| v.as_str()) {
                if text.len() > 100 {
                    format!("{}...", &text[..97])
                } else {
                    text.to_string()
                }
            } else if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
                if content.len() > 100 {
                    format!("{}...", &content[..97])
                } else {
                    content.to_string()
                }
            } else {
                format!("Object with {} fields", obj.len())
            }
        }
        Value::Array(arr) => {
            format!("Array with {} items", arr.len())
        }
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

// Enhanced text extraction with proper tool parsing
fn extract_enhanced_content(content: &[MessageContent]) -> (String, Vec<RichToolUse>, Vec<RichToolResult>) {
    let mut extracted_text: Vec<String> = Vec::new();
    let mut tool_uses: Vec<RichToolUse> = Vec::new();
    let mut tool_results: Vec<RichToolResult> = Vec::new();
    
    for (i, content_item) in content.iter().enumerate() {
        match content_item {
            MessageContent::Text(text) => {
                log(&format!("📝 Item {}: Text content: '{}'", i, text));
                extracted_text.push(text.clone());
            }
            MessageContent::ToolUse(tool_use) => {
                log(&format!("🔧 Item {}: Tool Use - {} ({})", i, tool_use.name, tool_use.id));
                
                // Parse the JsonData input
                let parsed_input = match parse_json_data(&tool_use.input) {
                    Ok(value) => value,
                    Err(e) => {
                        log(&format!("⚠️ Failed to parse tool use input: {}", e));
                        Value::Object(serde_json::Map::new())
                    }
                };
                
                let rich_tool_use = RichToolUse {
                    id: tool_use.id.clone(),
                    name: tool_use.name.clone(),
                    input: parsed_input,
                    status: ToolStatus::Pending,
                };
                
                tool_uses.push(rich_tool_use);
                
                // Add readable text representation
                extracted_text.push(format!("🔧 **Tool Call**: {}", tool_use.name));
            }
            MessageContent::ToolResult(tool_result) => {
                log(&format!("📊 Item {}: Tool Result for tool {}", i, tool_result.tool_use_id));
                
                // Parse the JsonData content
                let parsed_content = match parse_json_data(&tool_result.content) {
                    Ok(value) => value,
                    Err(e) => {
                        log(&format!("⚠️ Failed to parse tool result content: {}", e));
                        Value::String(format!("Error parsing result: {}", e))
                    }
                };
                
                let rich_tool_result = RichToolResult {
                    tool_use_id: tool_result.tool_use_id.clone(),
                    content: parsed_content.clone(),
                    is_error: tool_result.is_error,
                    execution_time_ms: None, // Could be extracted from metadata if available
                };
                
                tool_results.push(rich_tool_result);
                
                // Add readable text representation
                let status_emoji = if tool_result.is_error { "❌" } else { "✅" };
                let content_preview = format_tool_result_preview(&parsed_content);
                extracted_text.push(format!("{} **Tool Result**: {}", status_emoji, content_preview));
            }
        }
    }
    
    let combined_text = extracted_text.join("\n");
    log(&format!("✅ Enhanced extraction complete: {} tools, {} results", tool_uses.len(), tool_results.len()));
    
    (combined_text, tool_uses, tool_results)
}

// Legacy function for backward compatibility
fn extract_text_content(content: &[MessageContent]) -> String {
    let (text, _, _) = extract_enhanced_content(content);
    text
}

// Handle responses from chat-state-proxy actor
fn handle_chat_proxy_response(
    state: &mut FrontChatState,
    response: ChatProxyResponse,
) -> Result<(), String> {
    match response {
        ChatProxyResponse::Success => {
            log("Received success response from chat-state-proxy");
            Ok(())
        }
        ChatProxyResponse::Error { message } => {
            log(&format!(
                "Received error from chat-state-proxy: {}",
                message
            ));

            // Send error to WebSocket clients
            let error_response = ServerResponse::Error {
                message: format!("Chat error: {}", message),
            };

            for connection_id in state.active_connections.keys() {
                if let Ok(ws_message) = create_websocket_message(&error_response) {
                    if let Err(e) = http_framework::send_websocket_message(
                        state.server_id,
                        *connection_id,
                        &ws_message,
                    ) {
                        log(&format!(
                            "Failed to send error to connection {}: {}",
                            connection_id, e
                        ));
                    }
                }
            }

            Ok(())
        }
        ChatProxyResponse::ChatStateActorId { actor_id: _ } => {
            log("Received chat state actor ID from proxy");
            // Not used in this context
            Ok(())
        }
        ChatProxyResponse::History { messages } => {
            log(&format!("Received history with {} messages from proxy", messages.len()));
            // This response is typically handled directly in the request context
            // but we can log it here for debugging
            Ok(())
        }
        ChatProxyResponse::Metadata {
            conversation_id: _,
            store_id: _,
        } => {
            log("Received metadata from proxy");
            // Not used in this context but could be useful
            Ok(())
        }
    }
}

// Supervisor handlers implementation
impl SupervisorHandlersGuest for Component {
    fn handle_child_error(
        state: Option<Vec<u8>>,
        params: (String, bindings::theater::simple::types::WitActorError),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (actor_id, error) = params;
        log(&format!("Child actor {} encountered an error", actor_id));

        if let Some(err_msg) = error.data {
            match serde_json::from_slice::<Value>(&err_msg) {
                Ok(json) => log(&format!("Error details: {:?}", json)),
                Err(e) => log(&format!("Failed to parse error details: {}", e)),
            }
        }

        // Handle the error - could restart child, notify user, etc.
        // For now, just log it and return the same state
        Ok((state,))
    }

    fn handle_child_exit(
        state: Option<Vec<u8>>,
        params: (String, Option<Vec<u8>>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (actor_id, exit_state) = params;
        log(&format!("Child actor {} exited", actor_id));

        if let Some(err_msg) = exit_state {
            match serde_json::from_slice::<Value>(&err_msg) {
                Ok(json) => log(&format!("Error details: {:?}", json)),
                Err(e) => log(&format!("Failed to parse error details: {}", e)),
            }
        }

        // Handle child exit - cleanup, restart if needed, etc.
        Ok((state,))
    }

    fn handle_child_external_stop(
        state: Option<Vec<u8>>,
        params: (String,),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (actor_id,) = params;
        log(&format!("Child actor {} externally stopped", actor_id));

        // Handle external stop
        Ok((state,))
    }
}

bindings::export!(Component with_types_in bindings);
