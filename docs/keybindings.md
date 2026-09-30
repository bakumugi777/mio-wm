# Key Bindings

[日本語版](keybindings-jp.md)

Every key binding invokes a shared Action; keyboard, mouse, and IPC do not implement
separate window-management behavior. Declaring any KDL `bind` replaces the entire
built-in key map, so declare every binding you want to retain.

## Default bindings

| Key | Action |
|---|---|
| `Super+Arrow` / `Super+H/J/K/L` | Move focus directionally |
| `Super+Ctrl+Arrow` / `Super+Ctrl+H/J/K/L` | Move the Camera by one viewport |
| `Super+Ctrl+Shift+H/J/K/L` | Move the Camera by one Grid cell |
| `Super+Shift+Arrow` | Move the window by one Grid cell |
| `Super+Ctrl+Shift+Arrow` | Resize the window by one Grid cell |
| `Super+Shift+H/J/K/L` | Set the next window placement direction |
| `Super+1`–`Super+9` | Set absolute zoom from 0.1 through 0.9 |
| `Super+0` | Set absolute zoom to 1.0 |
| `Super+N` | Switch the active output |
| `Super+F` | Toggle floating |
| `Super+Enter` | Toggle fullscreen |
| `Super+M` | Toggle maximized |
| `Super+Z` | Toggle initial/half width |
| `Super+Q` | Close the window |
| `Super+O` | Toggle opacity |
| `Super+Shift+O` | Clear the runtime opacity override |
| `Super+B` | Toggle blur |
| `Super+W` | Toggle the cursor wake |
| `Super+V` | Toggle overview |
| `Super+S` | Confirm overview selection |
| `Super+R` | Reload the configuration |

## KDL example

```kdl
bind "Super+H" "focus-left"
bind "Super+Ctrl+H" "camera-left"
bind "Super+Shift+Left" "move-left"
bind "Super+5" "camera-zoom" 0.5
bind "Super+Q" "close"
```

Join `Ctrl`, `Alt`, `Shift`, `Super`, and a key with `+`. A chord may not be declared
twice. See the [configuration reference](configuration.md) for available Actions and
value ranges.
