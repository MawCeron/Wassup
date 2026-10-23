# Visual identity

Wassup is an unofficial WhatsApp client, so its look must evoke WhatsApp
without being mistaken for it, and it must feel native in GNOME and
libadwaita.

## Concept

The name plays on *what's up*, and the **shaka** hand (🤙) illustrates it:
a casual, friendly greeting. The app icon combines three ideas:

- **Messaging:** a rectangular speech bubble.
- **WhatsApp family:** a recognizable green, but not the same one.
- **Personality:** the shaka hand rotated as if holding a phone (thumb to
  the ear, pinky to the mouth).

## App icon

| File | Use |
|---|---|
| `data/icons/hicolor/scalable/apps/io.github.MawCeron.Wassup.svg` | Color icon, 128×128 |
| `data/icons/hicolor/symbolic/apps/io.github.MawCeron.Wassup-symbolic.svg` | Monochrome icon for the tray, 16×16 |

### Color icon

| Element | Specification |
|---|---|
| Canvas | 128×128 px |
| Safe margin | 8 px on every side (the bubble is 112 px wide) |
| Bubble | Rounded rectangle with the tail at the top left |
| Fill | Vertical linear gradient from `#46c364` (top) to `#1f8a3d` (bottom) |
| Hand | Shaka silhouette, `#ffffff`, rotated 60° clockwise, centered in the bubble body |
| Shadow | Three copies of the bubble shifted 1.0, 1.8 and 2.6 px down, black at 12%, 9% and 7% opacity |

The shadow is stacked layers rather than a blur filter, so it renders the
same everywhere. The green is deliberately darker than WhatsApp's
`#25d366`.

Checked at 128, 64, 32 and 16 px on light and dark backgrounds. The
fingers stay distinct down to 32 px; at 16 px the silhouette still reads.

### Symbolic icon

Only the hand silhouette, with the same 60° rotation and no bubble, filling
15×15 px of a 16×16 canvas. A version with the hand cut out of the bubble
was tried and dropped: at 16 px the fingers blur together.

## Interface icons

All of them live in `data/icons/hicolor/symbolic/actions/`.

| Icon name | Meaning | Size |
|---|---|---|
| `wassup-tick-symbolic` | Message sent | 16×16 |
| `wassup-double-tick-symbolic` | Message delivered or read | 22×16 |
| `wassup-send-symbolic` | Send message | 16×16 |
| `wassup-media-symbolic` | Photos and videos | 16×16 |
| `wassup-stickers-symbolic` | Stickers | 16×16 |
| `wassup-voice-note-symbolic` | Voice notes (microphone) | 16×16 |

Rules for any new interface icon:

- 16×16 grid; the double tick is the only wider one.
- Flat: no gradients, shadows or fixed colors.
- Line weight 1.5 px (1.8 px for the ticks), round caps and joins.
- Outline style, except the send arrow and small dots (eyes, sun), which
  are solid.

### Why the installed icons have no strokes

GTK recolors symbolic icons by setting the *fill* of their shapes. A
stroked path with `fill="none"` would be filled in solid. The installed
files are therefore the same drawings converted to filled outlines.
The editable stroked originals, which use `currentColor`, are in
`data/icons/src/`. Edit those and convert them back to outlines (for
example with Inkscape's *Path → Stroke to Path*) before installing.

### Message status

| Status | Icon | Style class |
|---|---|---|
| Sent | `wassup-tick-symbolic` | `dim-label` |
| Delivered | `wassup-double-tick-symbolic` | `dim-label` |
| Read | `wassup-double-tick-symbolic` | `accent` |

These are the same classes `show_status` in `ui/src/window.rs` already
applies to the text marks, so the icons can replace them without new CSS.

## Chat background

| File | Use |
|---|---|
| `data/backgrounds/wassup-pattern-dark.svg` | Tile for dark mode: white strokes at 7% opacity |
| `data/backgrounds/wassup-pattern-light.svg` | Tile for light mode: black strokes at 8% opacity |

A 576×576 px seamless tile of original line doodles: the shaka hand, chat
bubbles, ticks and a beach and surf theme (palm tree, wave, surfboard,
shell, sun). The background is transparent, so it works over any color.
Strokes are 1.5 px with round caps, like the interface icons.

The doodles are drawn for Wassup and do not copy WhatsApp's wallpaper.
GTK CSS cannot switch images with the color scheme, so apply one of two
classes from Rust according to `adw::StyleManager::is_dark()`:

```css
.chat-bg-dark  { background-image: url("<path to wassup-pattern-dark.svg>");  background-repeat: repeat; }
.chat-bg-light { background-image: url("<path to wassup-pattern-light.svg>"); background-repeat: repeat; }
```

Change the opacity on the root `<g>` of each file to make it more or less
visible.

## Trying the icons during development

```sh
mkdir -p ~/.local/share/icons
cp -r data/icons/hicolor ~/.local/share/icons/
gtk4-update-icon-cache -f -t ~/.local/share/icons/hicolor
```

The `.desktop` file in `packaging/` already points at the app icon by
name (`Icon=io.github.MawCeron.Wassup`).

## What to avoid

- The WhatsApp logo's layout: green circle, white ring, tail at the bottom
  left, white symbol inside. Wassup's bubble, tail position and hand are
  its own.
- `#25d366` as the main color.
- Meta or WhatsApp logos and names in the icon or in screenshots.
- Changing the hand's angle between versions: the 60° rotation is part of
  the identity.

Wassup is not affiliated with Meta or WhatsApp. Say so in the README, in
store descriptions and in the AppStream metainfo.
