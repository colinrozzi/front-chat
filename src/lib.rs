#[allow(warnings)]
mod bindings;

use bindings::exports::theater::simple::actor::Guest;
use bindings::exports::theater::simple::http_handlers::Guest as HttpHandlersGuest;
use bindings::exports::theater::simple::message_server_client::Guest as MessageServerClientGuest;
use bindings::exports::theater::simple::supervisor_handlers::Guest as SupervisorHandlersGuest;
use bindings::theater::simple::http_framework::{self, HandlerId, ServerId};
use bindings::theater::simple::http_types::{HttpRequest, HttpResponse, ServerConfig};
use bindings::theater::simple::message_server_host::{open_channel, send};
use bindings::theater::simple::random::generate_uuid;
use bindings::theater::simple::runtime::log;
use bindings::theater::simple::supervisor::spawn;
use bindings::theater::simple::types::{ChannelAccept, ChannelId};
use bindings::theater::simple::websocket_types::{MessageType, WebsocketMessage};
use serde::{Deserialize, Serialize};
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

// Server response protocol
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
    #[serde(rename = "error")]
    Error { message: String },
    #[serde(rename = "connected")]
    Connected { conversation_id: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChatMessage {
    id: String,
    role: String, // "user" or "assistant"
    content: String,
    timestamp: u64,
    finished: Option<bool>,
}

// Chat-state-proxy actor message protocol
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ChatStateRequest {
    #[serde(rename = "add_message")]
    AddMessage { message: ChatStateMessage },
    #[serde(rename = "generate_completion")]
    GenerateCompletion,
    #[serde(rename = "get_history")]
    GetHistory,
    #[serde(rename = "get_settings")]
    GetSettings,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
enum ChatStateResponse {
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "chat_message")]
    ChatMessage { message: ChatStateMessage },
    #[serde(rename = "history")]
    History { messages: Vec<ChatStateMessage> },
    #[serde(rename = "error")]
    Error { error: ErrorInfo },
}

// Simple message format for chat-state communication
#[derive(Serialize, Deserialize, Debug, Clone)]
struct ChatStateMessage {
    pub role: String, // "user" or "assistant"
    pub content: Vec<MessageContent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type")]
enum MessageContent {
    #[serde(rename = "text")]
    Text { text: String },
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ErrorInfo {
    pub code: String,
    pub message: String,
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

// --- Helper Functions ---

fn create_websocket_message(response: &ServerResponse) -> Result<WebsocketMessage, String> {
    let json_text = serde_json::to_string(response)
        .map_err(|e| format!("Failed to serialize response: {}", e))?;

    Ok(WebsocketMessage {
        ty: MessageType::Text,
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
        http_framework::enable_websocket(
            server_id, "/ws", None,       // connect handler (optional)
            ws_handler, // message handler (required)
            None,       // disconnect handler (optional)
        )
        .map_err(|e| format!("Failed to enable WebSocket: {}", e))?;

        // Spawn chat-state-proxy actor
        log("Spawning chat-state-proxy actor...");
        let chat_state_manifest = "/Users/colinrozzi/work/actor-registry/chat-state-proxy/manifest.toml";

        // Pass through the init data we received to the chat-state-proxy
        // This allows external control of the configuration
        let init_bytes = match &data {
            Some(bytes) => {
                log("Using provided init data for chat-state-proxy");
                bytes.clone()
            }
            None => {
                log("No init data provided, using empty config for chat-state-proxy");
                vec![]
            }
        };

        let chat_state_id = match spawn(chat_state_manifest, if init_bytes.is_empty() { None } else { Some(&init_bytes) }) {
            Ok(id) => {
                log(&format!("Successfully spawned chat-state-proxy actor: {}", id));

                // Open a channel with chat-state-proxy for real-time updates
                log("Opening channel with chat-state-proxy for real-time updates...");
                let subscribe_message = serde_json::json!({
                    "type": "channel_subscribe",
                    "channel": format!("conversation_{}", conversation_id)
                });

                let subscribe_bytes = serde_json::to_vec(&subscribe_message)
                    .map_err(|e| format!("Failed to serialize subscribe message: {}", e))?;

                match open_channel(&id, &subscribe_bytes) {
                    Ok(channel_id) => {
                        log(&format!(
                            "Successfully opened channel with chat-state: {}",
                            channel_id
                        ));
                        // We'll store the channel ID in state later when we can update it
                    }
                    Err(e) => {
                        log(&format!("Failed to open channel with chat-state-proxy: {}", e));
                        // Continue without channel - we'll still get direct message responses
                    }
                }

                Some(id)
            }
            Err(e) => {
                log(&format!("Failed to spawn chat-state-proxy actor: {}", e));
                // Continue without chat-state-proxy for now
                None
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
        let initial_state = FrontChatState {
            server_id,
            chat_state_id,
            conversation_id,
            active_connections: HashMap::new(),
            chat_state_channel: None, // Will be set when channel is established
        };

        Ok((Some(set_state(&initial_state)),))
    }
}

impl HttpHandlersGuest for Component {
    fn handle_request(
        state_bytes: Option<Vec<u8>>,
        params: (HandlerId, HttpRequest),
    ) -> Result<(Option<Vec<u8>>, (HttpResponse,)), String> {
        let (_handler_id, request) = params;

        log(&format!(
            "Handling {} request to {}",
            request.method, request.uri
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

        Ok((Some(set_state(&state)),))
    }

    fn handle_websocket_message(
        state: Option<Vec<u8>>,
        params: (HandlerId, u64, WebsocketMessage),
    ) -> Result<(Option<Vec<u8>>, (Vec<WebsocketMessage>,)), String> {
        let (_handler_id, connection_id, message) = params;

        let mut state = get_state(&state)?;

        match message.ty {
            MessageType::Text => {
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
        state.active_connections.remove(&connection_id);

        Ok((Some(set_state(&state)),))
    }
}

// --- Client Request Handling ---

fn handle_client_request(
    state: &mut FrontChatState,
    request: ClientRequest,
) -> Result<Vec<WebsocketMessage>, String> {
    match request {
        ClientRequest::SendMessage { content } => {
            log(&format!("Processing send_message: {}", content));

            // Create user message for display
            let user_message = ChatMessage {
                id: generate_uuid().map_err(|e| format!("Failed to generate message ID: {}", e))?,
                role: "user".to_string(),
                content: content.clone(),
                timestamp: 0, // TODO: Get actual timestamp
                finished: Some(true),
            };

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

                // Create chat-state message format
                let chat_state_message = ChatStateMessage {
                    role: "user".to_string(),
                    content: vec![MessageContent::Text { text: content }],
                };

                // Send add_message request
                let add_message_request = ChatStateRequest::AddMessage {
                    message: chat_state_message,
                };

                let request_json = serde_json::to_string(&add_message_request)
                    .map_err(|e| format!("Failed to serialize chat-state request: {}", e))?;

                // Send message to chat-state actor
                if let Err(e) = send(&chat_state_id, request_json.as_bytes()) {
                    log(&format!("Failed to send add_message to chat-state-proxy: {}", e));
                } else {
                    log("Successfully sent add_message to chat-state-proxy");

                    // Now request completion
                    let completion_request = ChatStateRequest::GenerateCompletion;
                    let completion_json = serde_json::to_string(&completion_request)
                        .map_err(|e| format!("Failed to serialize completion request: {}", e))?;

                    if let Err(e) = send(&chat_state_id, completion_json.as_bytes()) {
                        log(&format!(
                            "Failed to send generate_completion to chat-state-proxy: {}",
                            e
                        ));
                    } else {
                        log("Successfully sent generate_completion to chat-state-proxy");
                    }
                }
            } else {
                log("No chat-state-proxy actor available, creating fallback response");

                // Fallback: create a simple response
                let assistant_message = ChatMessage {
                    id: generate_uuid()
                        .map_err(|e| format!("Failed to generate message ID: {}", e))?,
                    role: "assistant".to_string(),
                    content: "Chat-state-proxy actor not available. Please try again.".to_string(),
                    timestamp: 0,
                    finished: Some(true),
                };

                let assistant_response = ServerResponse::MessageAdded {
                    message: assistant_message,
                };

                response_messages.extend(broadcast_to_connections(state, &assistant_response)?);
            }

            Ok(response_messages)
        }
        ClientRequest::GetConversation => {
            log("Processing get_conversation");

            // TODO: Get actual conversation from chat-state
            // For now, return empty conversation
            let response = ServerResponse::ConversationState {
                messages: vec![],
                conversation_id: state.conversation_id.clone(),
            };

            broadcast_to_connections(state, &response)
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
            match serde_json::from_str::<ChatStateResponse>(&message_str) {
                Ok(response) => match handle_chat_state_response(&mut state, response) {
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

            // Try to parse as a chat-state response
            match serde_json::from_str::<ChatStateResponse>(&message_str) {
                Ok(response) => {
                    log("Received chat-state response via channel");
                    match handle_chat_state_response(&mut state, response) {
                        Ok(_) => log("Successfully handled channel chat-state response"),
                        Err(e) => log(&format!(
                            "Error handling channel chat-state response: {}",
                            e
                        )),
                    }
                }
                Err(e) => {
                    log(&format!(
                        "Channel message is not a chat-state response: {}",
                        e
                    ));

                    // Try to parse as a generic update message
                    if let Ok(update) = serde_json::from_str::<serde_json::Value>(&message_str) {
                        log(&format!("Received channel update: {:?}", update));

                        // Forward channel updates to WebSocket clients
                        let server_response = ServerResponse::MessageUpdate {
                            message: ChatMessage {
                                id: generate_uuid().unwrap_or_else(|_| "update".to_string()),
                                role: "system".to_string(),
                                content: format!("Update: {}", update),
                                timestamp: 0,
                                finished: Some(true),
                            },
                        };

                        // Broadcast to all active connections
                        for connection_id in state.active_connections.keys() {
                            if let Ok(ws_message) = create_websocket_message(&server_response) {
                                if let Err(e) = http_framework::send_websocket_message(
                                    state.server_id,
                                    *connection_id,
                                    &ws_message,
                                ) {
                                    log(&format!(
                                        "Failed to send channel update to connection {}: {}",
                                        connection_id, e
                                    ));
                                }
                            }
                        }
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

// Handle responses from chat-state actor
fn handle_chat_state_response(
    state: &mut FrontChatState,
    response: ChatStateResponse,
) -> Result<(), String> {
    match response {
        ChatStateResponse::ChatMessage { message } => {
            log(&format!(
                "Received chat message from chat-state: {:?}",
                message
            ));

            // Convert chat-state message to our display format
            let display_message = ChatMessage {
                id: generate_uuid().map_err(|e| format!("Failed to generate message ID: {}", e))?,
                role: message.role,
                content: message
                    .content
                    .into_iter()
                    .filter_map(|c| match c {
                        MessageContent::Text { text } => Some(text),
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                timestamp: 0, // TODO: Get actual timestamp
                finished: Some(true),
            };

            // Broadcast to all WebSocket connections
            let server_response = ServerResponse::MessageAdded {
                message: display_message,
            };

            // Send to all active connections
            for connection_id in state.active_connections.keys() {
                if let Ok(ws_message) = create_websocket_message(&server_response) {
                    if let Err(e) = http_framework::send_websocket_message(
                        state.server_id,
                        *connection_id,
                        &ws_message,
                    ) {
                        log(&format!(
                            "Failed to send WebSocket message to connection {}: {}",
                            connection_id, e
                        ));
                    }
                }
            }

            Ok(())
        }
        ChatStateResponse::Success => {
            log("Received success response from chat-state");
            Ok(())
        }
        ChatStateResponse::Error { error } => {
            log(&format!(
                "Received error from chat-state: {} - {}",
                error.code, error.message
            ));

            // Send error to WebSocket clients
            let error_response = ServerResponse::Error {
                message: format!("Chat error: {}", error.message),
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
        ChatStateResponse::History { messages: _ } => {
            log("Received history response from chat-state (not implemented yet)");
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
        log(&format!(
            "Child actor {} encountered error: {:?}",
            actor_id, error
        ));

        // Handle the error - could restart child, notify user, etc.
        // For now, just log it and return the same state
        Ok((state,))
    }

    fn handle_child_exit(
        state: Option<Vec<u8>>,
        params: (String, Option<Vec<u8>>),
    ) -> Result<(Option<Vec<u8>>,), String> {
        let (actor_id, _exit_state) = params;
        log(&format!("Child actor {} exited", actor_id));

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

