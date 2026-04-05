# Commands

Commands are typed at the start of a message using the `/` prefix. Press `/` to open the command suggestion menu, then Tab to complete or Enter to select.

## /screen

Captures your screen and attaches it as context for the current message.

**Usage:** `/screen [optional message]`

**Examples:**
- `/screen`: sends a screenshot with no additional message
- `/screen what is this error?`: attaches a screenshot and asks the question

**Behavior:** The screenshot is taken the moment you press Enter. Thuki's own window is excluded from the capture: no flicker, no hide. The image appears in your message bubble exactly like a pasted screenshot.

**Limit:** One `/screen` capture per message. You may also attach up to 3 images manually (paste, drag, or the camera button) for a total of 4 images per message.

**Permission:** Requires Screen Recording permission. On first use, macOS will prompt you to grant it. If denied, Thuki cannot capture the screen. Grant access in System Settings > Privacy & Security > Screen Recording.
