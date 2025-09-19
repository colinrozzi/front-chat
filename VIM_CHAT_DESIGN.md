# Vim-Style Chat Interface Design

## Overview

This document outlines the design for a revolutionary chat interface that treats conversations like a vim text editor, where each message is an interactive component that can be navigated, selected, and edited using vim-style commands.

## Core Philosophy

Traditional chat interfaces are **append-only** - you send a message and it's static. Our approach treats the entire conversation as a **living document** where every message is editable, navigable, and interactive.

Think of it as: **"What if your entire chat history was a vim buffer?"**

## Three-Mode Hierarchy

The interface operates in three distinct modes, forming a hierarchy:

```
GLOBAL MODE (Top Level)
├── Navigate between messages
├── Select messages
└── Create new messages

    MESSAGE MODE (Message Selected)
    ├── Navigate within message content  
    ├── Message-level operations
    └── Enter editing mode

        EDITING MODE (Text Input)
        ├── Direct text editing
        ├── Vim text manipulation
        └── Content modification
```

## Mode Transitions

### GLOBAL → MESSAGE
- **Trigger**: `Enter` on selected message
- **Visual**: Message gets selection border
- **Behavior**: Focus shifts to within the message

### MESSAGE → EDITING  
- **Triggers**: `i, a, I, A, o, O` (vim insert commands)
- **Visual**: Message shows editing interface (textarea overlay)
- **Behavior**: Direct text input becomes available

### Escape Chain
- **EDITING** → `Esc` → **MESSAGE** → `Esc` → **GLOBAL**
- Always moves up one level in the hierarchy

## Command Mapping

### GLOBAL MODE Commands
| Key | Action | Description |
|-----|--------|-------------|
| `j` | Select Next | Move selection down to next message |
| `k` | Select Previous | Move selection up to previous message |
| `G` | Go to End | Select last message in conversation |
| `gg` | Go to Start | Select first message in conversation |
| `Enter` | Enter Message | Switch to MESSAGE mode on selected message |
| `o` | New Message | Create new message after current |
| `O` | New Message Above | Create new message before current |
| `/` | Search | Search through message content |
| `n` | Next Match | Jump to next search result |
| `dd` | Delete Message | Remove selected message from conversation |

### MESSAGE MODE Commands
| Key | Action | Description |
|-----|--------|-------------|
| `Esc` | Exit to Global | Return to GLOBAL mode |
| `i` | Insert | Enter EDITING mode at cursor position |
| `a` | Append | Enter EDITING mode after cursor |
| `I` | Insert Start | Enter EDITING mode at line beginning |
| `A` | Append End | Enter EDITING mode at line end |
| `o` | Open Below | Create new line below and enter EDITING |
| `O` | Open Above | Create new line above and enter EDITING |
| `j/k` | Navigate Lines | Move cursor within message content |
| `h/l` | Navigate Chars | Move cursor left/right within line |
| `w/b` | Word Motion | Jump between words in message |
| `dd` | Delete Line | Remove current line from message |
| `yy` | Yank Line | Copy current line |
| `p` | Paste | Paste copied content |

### EDITING MODE Commands
| Key | Action | Description |
|-----|--------|-------------|
| `Esc` | Exit to Message | Return to MESSAGE mode, save changes |
| `Ctrl+C` | Cancel | Return to MESSAGE mode, discard changes |
| Text Input | Direct Entry | Normal typing and editing |
| Arrow Keys | Navigation | Move cursor within text |
| Standard shortcuts | Cut/Copy/Paste | Normal text editing operations |

## Message Component Architecture

### Message States
```javascript
const MessageState = {
  DRAFT: 'draft',       // Being composed, no ID yet
  COMMITTED: 'committed', // Saved to conversation chain
  EDITING: 'editing',   // Being modified
  SELECTED: 'selected'  // Currently selected in MESSAGE mode
};
```

### Message Class Structure
```javascript
class Message {
  constructor(content, id = null, parentId = null) {
    this.content = content;
    this.id = id;              // null = draft, string = committed
    this.parentId = parentId;   // For conversation threading
    this.state = id ? 'committed' : 'draft';
    this.isSelected = false;
    this.cursorPosition = { line: 0, col: 0 };
    this.element = this.createElement();
  }
  
  // State management
  select() { /* Visual selection logic */ }
  deselect() { /* Remove selection */ }
  enterEditMode(insertType) { /* Setup editing interface */ }
  exitEditMode(save = true) { /* Cleanup and save/discard */ }
  
  // Vim command handlers
  handleVimCommand(cmd) { /* Command dispatch */ }
  moveCursor(direction) { /* Cursor navigation */ }
  deleteContent(range) { /* Content deletion */ }
  insertContent(text, position) { /* Content insertion */ }
}
```

## Visual Design System

### Selection States
- **No Selection**: `border: 1px solid transparent`
- **Selected (MESSAGE mode)**: `border: 2px solid #4A90E2; box-shadow: 0 0 0 1px #4A90E2`
- **Editing (EDITING mode)**: `border: 2px solid #7ED321; box-shadow: 0 0 0 1px #7ED321`

### Mode Indicators
- **Status Bar**: Shows current mode (GLOBAL | MESSAGE | EDITING)
- **Cursor**: Visible in MESSAGE mode, hidden in GLOBAL
- **Command Preview**: Show pending commands (e.g., "d" waiting for "d")

### Responsive Behavior
- **Auto-scroll**: Selected message centers in viewport
- **Keyboard Focus**: Always follows the current mode
- **Visual Feedback**: Immediate response to all commands

## Technical Implementation

### Event Handling
```javascript
class VimChatInterface {
  constructor() {
    this.mode = 'GLOBAL';
    this.selectedMessageIndex = 0;
    this.messages = [];
    this.commandBuffer = '';
    
    // Single global event listener
    document.addEventListener('keydown', this.handleKeydown.bind(this));
  }
  
  handleKeydown(event) {
    // Prevent default browser behavior for vim commands
    if (this.isVimCommand(event.key)) {
      event.preventDefault();
    }
    
    // Route to appropriate handler based on current mode
    switch(this.mode) {
      case 'GLOBAL': return this.handleGlobalCommand(event);
      case 'MESSAGE': return this.handleMessageCommand(event);
      case 'EDITING': return this.handleEditingCommand(event);
    }
  }
}
```

### Message Synchronization
- **Local Changes**: Immediate UI updates
- **Server Sync**: Debounced save to chat-state actor
- **Conflict Resolution**: Last-write-wins with visual indicators
- **History Tracking**: Undo/redo for message changes

## Advanced Features (Future)

### Conversation Branching
- Edit historical message → creates new conversation branch
- Visual tree navigation between branches
- Git-like merge capabilities

### Multi-Message Operations
- Visual selection across multiple messages
- Bulk operations (delete, move, format)
- Block-level editing commands

### Search and Replace
- `/pattern` - Search through all messages
- `:s/old/new/g` - Replace within selected message
- `:%s/old/new/g` - Replace across entire conversation

### Macros and Automation
- `qa` - Record macro to register 'a'
- `@a` - Replay macro
- Complex text transformations on messages

## Benefits

### For Power Users
- **Keyboard-Only Navigation**: Never touch the mouse
- **Rapid Text Manipulation**: Vim's editing power applied to chat
- **Conversation Archaeology**: Easy navigation through long conversations
- **Precision Editing**: Edit any message at any time with surgical precision

### for Developers
- **Component Architecture**: Each message is self-contained
- **Event System**: Clean separation between navigation and editing
- **Extensible Commands**: Easy to add new vim-style operations
- **State Management**: Clear state transitions and boundaries

### For Conversations
- **Living Documents**: Conversations evolve rather than append
- **Error Correction**: Fix typos and clarify meaning retroactively  
- **Thought Refinement**: Edit and improve ideas over time
- **Collaborative Editing**: Multiple people can refine shared understanding

## Implementation Phases

### Phase 1: Basic Three-Mode System
- [ ] Mode switching (GLOBAL ↔ MESSAGE ↔ EDITING)
- [ ] Basic navigation (j/k in GLOBAL, i/a/Esc transitions)
- [ ] Visual selection indicators
- [ ] Simple text editing

### Phase 2: Core Vim Commands
- [ ] Full navigation command set (w/b/gg/G/f/t)
- [ ] Text manipulation (dd/yy/p/x/r)
- [ ] Insert mode variants (I/A/o/O)
- [ ] Command history and repetition

### Phase 3: Advanced Features
- [ ] Search functionality (/ and ?)
- [ ] Multi-line operations
- [ ] Undo/redo system
- [ ] Macro recording and playback

### Phase 4: Conversation Features
- [ ] Message threading and branching
- [ ] Real-time collaboration
- [ ] Conflict resolution
- [ ] Export/import capabilities

## Why This Matters

This design represents a fundamental shift in how we think about digital conversations:

1. **From Static to Dynamic**: Messages become living, editable entities
2. **From Linear to Navigable**: Conversations become explorable spaces  
3. **From Mouse to Keyboard**: Efficiency through vim-style commands
4. **From Individual to Collaborative**: Shared editing of conversational meaning

The result is a chat interface that feels more like a powerful text editor than a traditional messaging app - perfect for technical discussions, collaborative thinking, and any scenario where precision and efficiency matter.

---

*This document is a living specification. As we implement and learn, we'll update these designs to reflect our discoveries and improvements.*
