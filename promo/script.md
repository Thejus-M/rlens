# rlens — promo script (9:16 · 1080×1920 · ~36 s)

**Goal:** in half a minute, show that rlens is Google Lens for the Linux desktop. Show the one
shortcut, translation in place, Lens search, and that it is free and works everywhere.
**Tone:** calm, confident, slightly playful. Dark UI, the four-colour Lens glow as the visual motif.
**Voice:** one warm, neutral narrator (local TTS). The captions show each word as it is spoken.
**Music:** a soft lofi bed at about 90 bpm. It ducks under the voice and rises on the end card.
**Demo content:** everything on screen is made up for the video, with no personal data.

| # | Time | Visual | Made with | On-screen text | Voice-over | Sound |
|---|---|---|---|---|---|---|
| 1 | 0.0–3.0 | Black screen. The rlens mark (a lens ring with a small glint) draws itself stroke by stroke in white, then a blue-red-yellow-green glow runs once around the ring. | Claude Drawing Skill | — | "Google Lens… on Linux?" | pen scratch, then a soft riser |
| 2 | 3.0–6.0 | Kinetic type, word by word, punching in on the beat: **Point. Read. Translate.** "rlens" settles below. | Bang Motion | Point. Read. Translate. / rlens | "Meet rlens." | three hits on the beat |
| 3 | 6.0–11.0 | Real screen: a busy desktop with a Spanish café poster. A key overlay shows `SUPER + SHIFT + L`. The screen freezes, the glow runs around the edges, and every word gets a soft highlight. | screen capture | `SUPER + SHIFT + L` | "One shortcut freezes your screen and reads every word." | key click, whoosh, shimmer |
| 4 | 11.0–18.0 | An animated ukiyo-e painting (waves, drifting clouds) with a Japanese poster banner. rlens is pressed and Translate is tapped: the Japanese glyphs fade out of the painting, and English appears in their place, in the same ink. | Huashu Art Motion + capture | Translate in place | "Translate anything — right where it is." | tap, then a soft "erase" swish |
| 5 | 18.0–23.0 | A product photo (headphones) on a page. Drag a box around it, tap Search, and the Google Lens panel slides in on the free side with results. | screen capture | Search what you see | "Search what you see." | drag, tap, slide |
| 6 | 23.0–27.0 | Quick pair: a QR code is decoded, Open link. Then a paragraph is selected and Copy shows "Text copied". | screen capture | Scan · Copy · Image | "Scan codes. Copy text. Grab the image." | two blips |
| 7 | 27.0–31.0 | Chips fly in and stack: Wayland · X11 · Hyprland · GNOME · KDE · Sway, then **Rust**. | Bang Motion | Every Linux desktop | "Every Linux desktop. Written in Rust." | quick ticks per chip |
| 8 | 31.0–36.0 | End card: the drawn mark returns, the glow ring pulses. **rlens**, `github.com/Thejus-M/rlens`, "Free · Open source · MIT". | Bang Motion + Claude Drawing | github.com/Thejus-M/rlens | "rlens. Free and open source." | impact, the music swells and ends |

## Voice-over (full text, ~70 words, about 30 s at a relaxed pace)

> Google Lens… on Linux?
> Meet rlens.
> One shortcut freezes your screen and reads every word.
> Translate anything — right where it is.
> Search what you see.
> Scan codes. Copy text. Grab the image.
> Every Linux desktop. Written in Rust.
> rlens. Free and open source.

## Captions

- The voice-over is transcribed with whisper word timestamps. Each word lights up in the brand
  accent as it is spoken (karaoke style).
- The captions sit in the lower third and stay off any UI the shot is showing.

## Pipeline notes

- **Real footage:** rlens is driven by the promo-studio uinput rig (`vmouse.py`), so the moves are
  smooth and repeatable. It is recorded at 60 fps and reframed to 9:16 with gentle push-ins.
- **Skills:** Claude Drawing Skill (beats 1 and 8), Huashu Art Motion (the painting in beat 4),
  and Bang Motion (beats 2, 7 and 8).
- **Delivery:** `rlens/promo/rlens-promo-9x16.mp4`, 1080×1920, 30 fps, h264 + aac, -14 LUFS.

## Style brief (Bang Motion, Law #3)

- **Theme / feeling:** a lens that reads the world. Curious, precise, quietly magical.
- **Concept (from the opener concept menu):** candidates were #9 *one feature, fully demoed*,
  #10 *diegetic question → answer* and #3 *chaos → gone*. **Chosen: #10 + #9.** The question
  "Google Lens… on Linux?" is written in the world (the drawn line), and rlens answers by acting
  on a real desk. Translate in place is the one feature shown in full. Search, QR and copy are
  quick flashes. Structure fingerprint: `diegetic question · 8 beats · line → title words → desk →
  painting → product → flashes → address → name`.
- **Palette:** night blue `#121a2e` (the line-art backdrop, inherited from the drawn mark) ·
  gold `#e8cc8a` (the drawing's line, derived: it is the mark's own ink) · the four rlens glow
  colours `#4285f4 #ea4335 #fbbc05 #34a853` (from the rlens overlay glow), used only as moving light.
- **Display font:** Space Grotesk 500, sentence case, with no full stops on short phrases.
- **Background language:** slow breathing grain plus one soft gold vignette; the glow colours
  travel only along edges and rings.
- **Motion signature:** the four-colour glow travelling around a shape (screen edge, lens ring,
  word underline). It is the same light rlens shows on screen.
- **Closing (from the closing menu):** *address → name*. The repo address is typed, erased from
  the left, and "rlens" is typed in its place, then a hold.
- Beat 7 (desktop chips) is replaced by real 0.4 s UI flashes: chips are banned by Bang Motion's
  Law #1.
