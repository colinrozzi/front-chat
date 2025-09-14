# Front Chat Actor

A WebSocket-enabled chat interface built with [Theater](https://github.com/colinrozzi/theater) WebAssembly actors. This actor provides a real-time chat interface that communicates with chat-state actors via the Theater message system.

## 🚀 Quick Start

### Prerequisites

- [Rust](https://rustup.rs/) (1.81.0 or newer)
- [Theater CLI](https://github.com/colinrozzi/theater) installed
- `cargo component` installed (`cargo install cargo-component`)

### Building the Component

```bash
# Build the WebAssembly component
cargo component build --release
```

### Running the Chat Server

```bash
# Start the server
theater start manifest.toml
```

The server will start on **http://localhost:8080**

## 🎭 Architecture

This front-chat actor implements a **one-actor-per-conversation** model that mirrors the chat-state architecture:

```
Browser (WebSocket) ↔ front-chat ↔ chat-state
                            ↑         ↓
                      Actor Messages  Channel Updates
```

### Core Features

- **WebSocket Real-time Communication**: Bidirectional real-time messaging
- **Modern Chat Interface**: Clean, responsive web interface
- **Actor Message Integration**: Direct communication with chat-state actors
- **Channel Subscriptions**: Real-time updates via Theater's channel system
- **Connection Management**: Automatic reconnection and error handling

## 🌐 Endpoints

### HTTP Endpoints
- `GET /` - Chat interface (HTML + WebSocket client)
- `GET /health` - JSON health check response

### WebSocket Endpoint
- `WS /ws` - Real-time chat communication

## 📨 WebSocket Protocol

### Client → Server Messages

```javascript
// Send a message
{
  "type": "send_message",
  "content": "Hello, world!"
}

// Get conversation history
{
  "type": "get_conversation"
}

// Update settings
{
  "type": "update_settings",
  "settings": {
    "temperature": 0.7,
    "model": "claude-3-5-sonnet"
  }
}
```

### Server → Client Messages

```javascript
// Connection established
{
  "type": "connected",
  "conversation_id": "conv_abc123"
}

// New message added
{
  "type": "message_added",
  "message": {
    "id": "msg_123",
    "role": "assistant",
    "content": "Hello back!",
    "timestamp": 1694723405,
    "finished": true
  }
}

// Conversation state
{
  "type": "conversation_state", 
  "messages": [...],
  "conversation_id": "conv_abc123"
}

// Error occurred
{
  "type": "error",
  "message": "Something went wrong"
}
```

## 🏗️ Project Structure

```
front-chat/
├── src/
│   ├── lib.rs             # Main actor implementation
│   └── bindings.rs        # Generated WIT bindings
├── wit/                   # WebAssembly Interface Types
├── chat.html             # Chat interface HTML/CSS/JS
├── Cargo.toml            # Rust dependencies
├── manifest.toml         # Theater actor configuration
└── README.md             # This file
```

## 🎨 Chat Interface Features

- **Real-time Messaging**: Instant message delivery via WebSocket
- **Typing Indicators**: Shows when AI is processing
- **Auto-scroll**: Automatically scrolls to new messages
- **Connection Status**: Visual connection state indicator
- **Responsive Design**: Works on desktop and mobile
- **Error Handling**: Graceful error display and recovery
- **Auto-reconnect**: Automatically reconnects on disconnect

## 🔧 Development

### Chat-State Integration

Currently implemented with a simple echo response. To integrate with actual chat-state actors:

1. **Actor Spawning**: Uncomment supervisor spawn functionality
2. **Message Passing**: Implement ChatStateRequest message sending
3. **Channel Subscription**: Subscribe to chat-state update channels
4. **Real-time Updates**: Forward channel updates to WebSocket clients

### Current Status

- ✅ WebSocket server implementation
- ✅ Modern chat interface
- ✅ Message protocol definition  
- ✅ Connection management
- ✅ Error handling
- 🔄 Chat-state actor integration (in progress)
- 🔄 Channel subscription system (in progress)

### Next Steps

1. **Actor Communication**: Integrate with chat-state message system
2. **Channel Subscriptions**: Listen for real-time chat-state updates  
3. **Settings Management**: Implement conversation settings UI
4. **Message History**: Load and display conversation history
5. **Streaming Responses**: Support for streaming AI completions

## 🚀 Deployment

The component generates optimized WASM files:

- **Debug**: `target/wasm32-wasip1/debug/front_chat.wasm`
- **Release**: `target/wasm32-wasip1/release/front_chat.wasm`

Deploy using Theater's actor system for scalable, distributed chat interfaces.

## 📚 Learn More

- [Theater Documentation](https://github.com/colinrozzi/theater)
- [WebAssembly Component Model](https://github.com/WebAssembly/component-model)
- [WIT (WebAssembly Interface Types)](https://github.com/WebAssembly/wit-bindgen)

---

*Built with ❤️ using Theater WebAssembly actors and modern web technologies*
