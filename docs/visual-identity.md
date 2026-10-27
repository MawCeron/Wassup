# Visual identity

Wassup is an unofficial WhatsApp client, so its look must evoke WhatsApp
without being mistaken for it, and it must feel native in GNOME and
libadwaita.

## Concept

The name plays on *what's up*. The app icon is a bold, rounded **W** with
two small marks that read as a wave or a spark, inside a speech bubble. It
combines three ideas:

- **Messaging:** a speech bubble with a short tail at the bottom left.
- **WhatsApp family:** a recognizable green, but not the same one.
- **Personality:** a round, friendly W whose last stroke ends in a flick,
  like a greeting. The shaka hand (🤙) that inspired the name lives on as a
  motif in the chat background.

## App icon

| File | Use |
|---|---|
| `data/icons/hicolor/scalable/apps/io.github.MawCeron.Wassup.svg` | Color icon, 128×128 |
| `data/icons/hicolor/symbolic/apps/io.github.MawCeron.Wassup-symbolic.svg` | Monochrome icon for the tray, 16×16 |

### Color icon

| Element | Specification |
|---|---|
| Canvas | 128×128 px, with every coordinate of the bubble on the 2 px grid of the GNOME app icon template |
| Margins | 12 px at the top, right and bottom; 8 px at the tail tip |
| Bubble | Rounded rectangle of 96×100 px with a 16 px corner radius and a short tail at the bottom left |
| Face | Flat `#2ec27e` (GNOME palette, Green 4) |
| Front profile | The same bubble shifted 4 px down, flat `#26a269` (Green 5). 4 px is the maximum depth the guidelines allow |
| Mark | The W and its two marks, `#ffffff`, centered horizontally in the face and 2 px above its vertical center |
| Shadow | None |

The guidelines ask for flat colors on straight surfaces and gradients only
on curved ones, so the face has no gradient. They also say the system draws
the outer shadow, so nothing is drawn outside the silhouette. The green is
close to the WhatsApp family but is not WhatsApp's `#25d366`.

Scale the W and its marks uniformly. Stretching them turns the round ends
of the strokes into ovals.

The bubble is not centered on the canvas: its body sits to the right so
that the tail keeps its margin. The visual center of mass ends up about 2%
right of the canvas center, which looks balanced because the tail is light.

Checked at 128, 64, 48, 32 and 16 px on light and dark backgrounds. The W
is clear down to 32 px; at 16 px the icon reads as a green bubble with a W.

### Symbolic icon

The W and its two marks as a single silhouette, without the bubble, about
13 px wide, centered on a 16×16 canvas. The marks are drawn 2 px wide so
they are not hairlines, and they keep at least 1 px of space between each
other and from the W. A W cut out of the bubble was dropped because it loses
its strokes.

## Interface icons

All of them live in `data/icons/hicolor/symbolic/actions/`.

| Icon name | Meaning | Size |
|---|---|---|
| `wassup-tick-symbolic` | Message sent | 16×16 |
| `wassup-double-tick-symbolic` | Message delivered or read | 16×16 |
| `wassup-send-symbolic` | Send message | 16×16 |
| `wassup-media-symbolic` | Photos and videos (also used in the attach menu) | 16×16 |
| `wassup-stickers-symbolic` | Stickers | 16×16 |
| `wassup-emoji-symbolic` | Emoji picker (round smiley) | 16×16 |
| `wassup-document-symbolic` | Attach a document (page with a folded corner) | 16×16 |
| `wassup-contact-symbolic` | Share a contact (head and shoulders) | 16×16 |
| `wassup-poll-symbolic` | Create a poll (three vertical bars) | 16×16 |
| `wassup-voice-note-symbolic` | Voice notes (microphone) | 16×16 |

Rules for any new interface icon, from the GNOME guidelines for UI icons:

- 16×16 canvas, no exceptions: the double tick fits in it too.
- 2 px strokes for the main features, never 1 px. Put stroke centers on
  whole or half pixels so that the edges of horizontal and vertical lines
  land on whole pixels.
- Round caps and joins.
- Flat and monochrome: no opacity, duotone, dithering, gradients or fixed
  colors, because the icon is recolored at runtime.
- One idea per icon, and a simple orthogonal view.
- Outline style, except for the send arrow, the mountains of the media icon
  and small dots (eyes, sun), which are solid.

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

`show_status` in `ui/src/window/rows.rs` applies them.

## Chat background

| File | Use |
|---|---|
| `data/backgrounds/wassup-pattern-dark.svg` | Tile for dark mode: white strokes at 7% opacity |
| `data/backgrounds/wassup-pattern-light.svg` | Tile for light mode: black strokes at 8% opacity |

A 576×576 px seamless tile of original line doodles: the shaka hand, chat
bubbles, ticks and a beach and surf theme (palm tree, wave, surfboard,
shell, sun). The background is transparent, so it works over any color.
Strokes are 1.5 px with round caps. This is a decorative pattern, not a UI icon, so the 2 px rule does not apply.

The doodles are drawn for Wassup and do not copy WhatsApp's wallpaper.
Both tiles are bundled into the UI binary with the icons. The `.chat-bg`
rule in `ui/src/window.rs` picks one with a `prefers-color-scheme` media
query, and in light mode tints the backdrop slightly so white received
bubbles stand out. The pattern stays fixed while messages scroll.

Change the opacity on the root `<g>` of each file to make it more or less
visible.

## Trying the icons during development

The interface icons are compiled into the UI binary as a GResource by
`ui/build.rs`, so they work without installing anything; rebuild after
changing them. The app icon needs installing:

```sh
mkdir -p ~/.local/share/icons
cp -r data/icons/hicolor ~/.local/share/icons/
gtk4-update-icon-cache -f -t ~/.local/share/icons/hicolor
```

The `.desktop` file in `packaging/` already points at the app icon by
name (`Icon=io.github.MawCeron.Wassup`).

## GNOME guidelines

Checked against the [app icon](https://developer.gnome.org/hig/guidelines/app-icons)
and [UI icon](https://developer.gnome.org/hig/guidelines/ui-icons.html)
guidelines and the [palette](https://developer.gnome.org/hig/reference/palette.html).

| Guideline | Status |
|---|---|
| App icon drawn in 128×128 px without filling the whole area | Met |
| Shapes on the template's 2 px grid | Met for the bubble; the W is an organic shape and is only centered |
| Unique silhouette, no extreme aspect ratio | Met |
| Front profile of at most 4 px, darker than the top surface | Met |
| Flat colors on straight surfaces, from the GNOME palette | Met |
| No shadows outside the silhouette | Met |
| Symbolic version of the app icon | Met |
| UI icons on a 16×16 canvas, with 2 px strokes and no hairlines | Met |
| UI icons aligned to the pixel grid | Met for horizontal and vertical edges; diagonals cannot align |
| No opacity or gradients in symbolic icons | Met |
| Bottom of the app icon on the template's baseline | Not checked: needs the template from App Icon Preview |
| Appearance in context | Not checked: preview with App Icon Preview and Symbolic Preview |

The guidelines also advise avoiding logos that depend on a specific visual
style. The W is a letter mark of the app's own name, and it does not copy
the WhatsApp logo.

## What to avoid

- The WhatsApp logo's layout: a green circle or tile with a white ring, a
  round bubble with the tail at the bottom left, a white symbol inside.
  Wassup's squarish bubble, short tail and W are its own.
- `#25d366` as the main color.
- Meta or WhatsApp logos and names in the icon or in screenshots.
- Stretching the W or changing its proportions: the round, even strokes
  are part of the identity.

Wassup is not affiliated with Meta or WhatsApp. Say so in the README, in
store descriptions and in the AppStream metainfo.
