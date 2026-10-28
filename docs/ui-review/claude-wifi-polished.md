# Quickshare Wi-Fi polish final check (Claude)

Read-only. Screenshots are in `wifi-clean/`. [V] = seen in a screenshot.

**Verdict: all three items are resolved. No remaining issues for this flow.**

1. **Password row: resolved [V].** `wifi-received.png` shows a plain row: "Password", eight dots (`••••••••`), and an eye toggle. There's no pencil, no greyed-out editor and no clipped glyphs. `wifi-revealed.png` shows "not-a-real-password" with the toggle switched to the hide (eye-slash) icon. The button shows keyboard focus after the click, which is expected. A fixed-length mask does not leak the password length. Good.
2. **Preview title: resolved [V].** At 360x640, `wifi-small.png` shows "Quickshare test netw…" on one line, the "Wi-Fi network" subtitle, and the edit and close buttons. "Nearby Devices" stays visible. I can't see the tooltip in a static capture, so I'm taking the parent's word that it shows the full SSID.
3. **Activity ticks: resolved in the capture [V].** The fresh `activity.png` has no marks between the header and "Your Transfers". The parent's trace (a clean render-node texture, and a full repaint that removes them) points to stale Xvfb/Cairo pixels, not app layout. That clears the app's widget tree and this harness. It does not prove anything about real Wayland or GPU rendering, and it doesn't change GTK's renderer. A one-time look on a real GNOME session would settle that, but it's not a blocker.

Not covered: physical Android/Windows interop, dark or high-contrast themes, and tooltip contents.
