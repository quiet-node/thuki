# Thuki Design System

This document is the single source of truth for Thuki's visual and interaction design.
Use the color tokens, typography, and motion principles exactly as specified.

---

## 1. Product Identity

**Name:** Thuki (pronounced "too-kee", from Vietnamese "thu ky" meaning secretary)

**Tagline:** _Your AI secretary. Always there. Always private._

**One-liner:** A floating AI overlay for macOS that you summon with a double-tap of
Control. Fully local. Zero cloud. Free forever.

**Personality:** Calm, minimal, precise. Not flashy. Not corporate. The vibe is a
premium tool that respects your focus and your privacy. Think: Arc browser meets
macOS Spotlight.

**Target audience:** Developers, power users, and anyone who wants fast AI answers
without leaving their current app or giving up their data.

**Core differentiator:** It floats above every app, including fullscreen ones. No
other AI tool does this.

---

## 2. Color Palette

Use these exact values. No substitution.

### Brand Colors

| Token     | Hex       | Role                              |
| --------- | --------- | --------------------------------- |
| Primary   | `#ff8d5c` | CTAs, accents, glow, highlights   |
| Secondary | `#bc5c0b` | Depth gradients, pressed states   |
| Tertiary  | `#e69c05` | Accent pops, secondary highlights |
| Neutral   | `#1a1a1c` | Background fallback               |

### Surface Colors

| Token            | Value                      | Role                                                |
| ---------------- | -------------------------- | --------------------------------------------------- |
| Surface Base     | `rgba(22, 18, 15, 0.98)`   | Main window background (near-black, warm undertone) |
| Surface Elevated | `rgba(38, 30, 24, 0.60)`   | Cards, elevated panels                              |
| Surface Border   | `rgba(255, 141, 92, 0.12)` | Window edge, subtle orange rim                      |

### Text Colors

| Token          | Hex       | Role                          |
| -------------- | --------- | ----------------------------- |
| Text Primary   | `#f0f0f2` | Body text, labels             |
| Text Secondary | `#8a8a8e` | Metadata, hints, placeholders |

### Glow / Ambient Effects

- Top edge of the window has a subtle horizontal gradient glow:
  `linear-gradient(90deg, transparent, rgba(255, 141, 92, 0.35), transparent)`
  spanning 70% of the width, 1px tall, sitting flush at the very top of the window frame.
- Background of the window has a soft radial ambient:
  `radial-gradient(ellipse 70% 40% at 50% 0%, rgba(255, 141, 92, 0.08) 0%, transparent 65%)`
- These two effects give the window a warm ember glow from the top. Critical to reproduce.

### Shadows

- **Bar mode:** `0 6px 20px -6px rgba(0,0,0,0.6), 0 0 0 1px rgba(255,141,92,0.12)`
- **Chat mode:** `0 4px 14px -3px rgba(0,0,0,0.5), 0 1px 4px -1px rgba(0,0,0,0.3), 0 0 0 1px rgba(255,141,92,0.12)`

---

## 3. Typography

**Primary font:** Inter (Google Fonts). Fall back to system-ui, -apple-system.

| Use                 | Weight  | Approx size |
| ------------------- | ------- | ----------- |
| Input text          | 400     | 15px        |
| Chat response body  | 400     | 14px        |
| UI labels, buttons  | 500     | 13px        |
| Section headers     | 600-700 | 24-36px     |
| Hero / display text | 700-800 | 48-72px     |

---

## 4. Window Dimensions and Layout

Thuki has two distinct visual states. Both are centered on screen (or near the cursor).

### Bar Mode (compact, Spotlight-style)

- Width: 600px logical
- Height: ~80px
- Border radius: 16px
- Content: Thuki logo (left) + text input (center) + send button (right)
- The window background is nearly opaque warm dark with the ambient top glow.
- Feels like macOS Spotlight: minimal, surgical, focused.

### Chat Mode (expanded)

- Width: 600px (same)
- Height: up to 648px (600px content + 48px padding)
- Border radius: 16px (same, no jarring reflow)
- Content: window chrome (top bar with controls, history, new-chat) + scrollable message list + input bar (bottom)
- The bar-to-chat transition is a smooth spring-based morph: the container grows downward.

### Window Controls (chat mode only)

Small row at top of chat window: close button (left), history button (center-left),
new conversation button (right). Minimal macOS-native feel.

---

## 5. Chat Bubble Design

### User Bubble

- Background: `linear-gradient(135deg, #ff8d5c 0%, #e06b30 100%)`
- Shadow: `0 2px 12px -2px rgba(255, 141, 92, 0.3), inset 0 1px 0 rgba(255,255,255,0.15)`
- Text color: white
- Alignment: right side
- Border radius: 12px (pill-ish on right, flat on bottom-right to indicate "sent")

### AI Bubble

- Background: `rgba(36, 30, 26, 0.95)` with `backdrop-filter: blur(12px)`
- Border: `1px solid rgba(255,255,255,0.06)`, top border slightly warmer `rgba(255,141,92,0.10)`
- Shadow: `0 2px 8px -2px rgba(0,0,0,0.3)`
- Text color: `#f0f0f2`
- Alignment: left side
- Renders markdown: code blocks have dark backgrounds with orange-tinted borders.

### Typing Indicator

Three dots, warm orange `#ff8d5c`, pulsing in sequence. Shown while AI is generating.

---

## 6. Motion and Animation Principles

**Framework in the real app:** Framer Motion (React). Replicate the feel, not the code.

### Key easing

The morphing container uses: `cubic-bezier(0.12, 0.8, 0.2, 1.18)` over `500ms`.
This is a slight overshoot spring: it grows just a hair past its target then settles.
Use this feel for major state transitions in the video.

### Principles

- **Minimal, purposeful motion.** Nothing bounces for the sake of bouncing.
- **Spring-forward, snap-back.** Entrances have a subtle overshoot. Exits are clean cuts or fast fades.
- **Stagger reveals.** When multiple elements appear (e.g., chat bubbles), stagger each by ~60-80ms.
- **No rotation effects.** No 3D flips, no spinning logos. Clean 2D.
- **Opacity + Y-translate for entrances.** Elements fade in while sliding up 8-12px.

### The signature morph

The bar expanding into chat mode is Thuki's signature interaction. The window grows
downward from 80px to ~640px over 500ms with the slight overshoot spring. This should
be shown prominently in the video and feel satisfying.

---

## 7. Logo

- Use on dark backgrounds only. The logo is designed for the warm-dark Thuki surface.
- In bar mode: appears at the left side of the input bar, smaller (24-32px).
- In onboarding and hero contexts: can be displayed larger (120-200px) centered.
- Do not add drop shadows or heavy glow effects to the logo itself.
