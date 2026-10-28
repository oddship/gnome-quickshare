# Quickshare high-contrast drop-zone re-check (Claude)

Read-only. Fresh Xvfb/Cairo captures are in `wifi-high-contrast-polished/` and `wifi-high-contrast-dark/`. [V] = seen in a screenshot.

**Verdict: resolved. No remaining theme issues in these captures.**

- **Light high contrast [V]:** the dashed drop-zone outline is now a clearly darker grey. It is about as visible as the solid HC borders on the Wi-Fi card and the "Share Text…" button. In the earlier capture it was pale and fainter than them.
- **Dark high contrast [V]:** the dashed outline is a clearly visible light grey on the dark background, about as strong as the card and button borders.
- **Received dialog (both HC variants) [V]:** unchanged and clean. Row borders, the masked password, the reveal and copy icons, and the bordered Copy Password and Close buttons all render correctly. Nothing is clipped. There is no drop area in the dialog, so the window-scoped class doesn't affect it.
- **Approach:** the AdwStyleManager `high-contrast` class plus a `.high-contrast .drop-area` override is a reasonable choice for the GTK 4.14 floor. The normal 0.25 rule is untouched. I didn't re-capture normal light or dark, so their "unchanged" status rests on the CSS diff as described.

Scope: visual QA only. No accessibility-compliance, measured-contrast, GPU/Wayland or physical-device claim.
