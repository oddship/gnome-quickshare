# Quickshare Wi-Fi dark / high-contrast check (Claude)

Read-only. Captures are in `wifi-dark/` and `wifi-high-contrast/` (Xvfb/Cairo). [V] = seen in a screenshot. This is a visual check only. It does not make any accessibility-compliance claim.

**Verdict: no theme regressions block the Wi-Fi flow.** Layout, wrapping and the single-line ellipsized preview title are identical to light in both themes. Nothing is clipped by the theme change. All text, icons, the switch, the spinner and the bottom bar render with the theme's own colours. High contrast adds borders to cards, rows, buttons and window controls as expected. The masked `••••••••` row and the reveal/copy icons are clearly visible in both.

## One concrete fix [V]
**The drop-zone border doesn't respond to high contrast.** In `wifi-high-contrast/wifi-small.png` the dashed drop-area border is the same pale grey as in light mode, fainter than the solid HC borders on the cards and buttons right below it. The cause is the fixed `alpha(@window_fg_color, 0.25)` in `data/style.css`. In dark it reads as a faint grey dash, which is acceptable.
- **Fix:** add a contrast override and keep the dash:
  ```css
  @media (prefers-contrast: more) {
    .drop-area { border-color: alpha(@window_fg_color, 0.6); }
  }
  ```
  (libadwaita ≥1.6 / GTK ≥4.16 supports `prefers-contrast`. On older stacks, use the `.hc`-style approach your Adwaita version provides.)
- **Pass:** in the HC capture the dashed outline is at least as visible as the solid card borders below it. Light and dark are unchanged.

## Note (not theme-specific, no action needed)
- In the Share Wi-Fi form, the masked password field is scrolled to the caret, so the first dot is cut at the left edge in all three themes. This is native `PasswordEntryRow` behaviour for a value longer than the field, not a regression.

Not covered: real GPU/Wayland rendering, physical devices, measured contrast ratios.
