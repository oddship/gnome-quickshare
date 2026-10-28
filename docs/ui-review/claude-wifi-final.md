# Quickshare Wi-Fi UI final re-check (Claude)

Read-only. Screenshots are in `wifi-final/`. [V] = seen in a screenshot. [H] = untested hypothesis.

**Verdict: pass. Nothing blocks the Wi-Fi flow.** Every ranked item from `claude-wifi.md` is fixed (#5 was optional and deferred).

## Verified [V]
- **Received dialog:** the heading names the sender ("Wi-Fi Network from Quickshare test peer"). It has Network, Security ("WPA/WPA2 Personal"), a masked Password with reveal, and Hidden Network rows. There's a copy button on Network, plus "Copy Password". After Copy, the "Password copied" toast appears.
- **Consent:** it reads `Wi-Fi network “Quickshare test network”`. Decline has initial focus.
- **Activity:** "Complete · From Quickshare test peer · 06:49", with no size.
- **Preview (360x640):** the subtitle says "Wi-Fi network". The remove button is the circular close icon. "Nearby Devices" is visible.
- **Form:** unchanged and still good. Choose Device is enabled once the input is valid.

## Remaining polish (non-blocking, ranked)
1. **The received password row looks like a disabled editor [V].** In `wifi-received.png` the read-only `PasswordEntryRow` (`set_editable(false)`, `src/window/wifi.rs:180-184`) still shows the greyed pencil edit indicator. The masked dots are cut mid-glyph against it. That reads as "you can't edit this" rather than "here is the password".
   - **Fix:** use an `AdwActionRow` titled "Password". Its subtitle is a masked string (e.g. `●`×len, capped at about 12). Add a flat `view-reveal-symbolic` toggle suffix that swaps the subtitle to the real password, with tooltip "Show Password"/"Hide Password". If you keep the entry row, at least hide the edit indicator with CSS (`row.entry image.edit-icon { opacity: 0 }` or similar) so there is no pencil.
   - **Pass:** no pencil icon on the password row. The dots are not clipped. Reveal still works.
2. **Stray marks under the header on Activity [V].** In `activity.png` there are three small grey ticks at about y=77 (x≈305, 448, 540), just under the switcher and above "Your Transfers". They look like the narrow-Send specks fixed earlier, likely leftovers of the switcher, a stack transition, or the scroll undershoot.
   - **Fix:** find the widget that draws them, for example with the GTK Inspector's widget outline at that spot.
   - **Pass:** no marks between the header and "Your Transfers".
3. **Optional [V]:** at 360 wide the SSID in the preview row wraps to two lines ("Quickshare test / network"). That's acceptable. `set_title_lines(1)` with ellipsis plus a full-SSID tooltip would keep the row compact. Low priority.

## Limits
No physical Android/Windows testing. Clipboard behaviour is taken from the parent's report of its Xvfb checks, not from screenshots. Dark and high-contrast themes were not captured.
