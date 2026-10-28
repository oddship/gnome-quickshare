# Quickshare Wi-Fi sharing UI review (Claude)

Read-only review of 7 GTK captures in `wifi-full/`. Source was read for context only (`src/window/wifi.rs`, `src/window/transfers.rs`).
[V] = seen in a screenshot. [H] = untested interaction or accessibility hypothesis.

**Verdict: nothing blocks this flow.** Composing, validating, the masked password, the preview, the matching-code consent, the review-before-use dialog, history without the payload, and the 360x640 layout all work as described. The items below are polish worth doing in this pass, ranked.

## What works [V]
- The form uses native rows (`EntryRow`, `ComboRow`, `PasswordEntryRow` with reveal, `SwitchRow`). "Choose Device" stays disabled until the input is valid. The body says the password is only sent after the other device accepts.
- The Send preview row (SSID, "Wi-Fi details · Select a nearby device", edit and clear) sits directly above Nearby Devices. At 360x640 "Nearby Devices" and the spinner are still visible.
- In the consent dialog Decline has initial focus and is the default/close response. The code is large.
- The Activity row says "Wi-Fi Details" with no SSID or password. That matches the privacy promise.

## Findings (ranked)

### 1. The received Wi-Fi dialog is a text dump: Copy grabs the whole blob, and security type is missing [V]
- **Evidence:** In `wifi-received.png`, under the heading "Something for you", there is a free-text box: "Network: … / Password: not-a-real-password / Hidden network: Yes", with a text cursor after "Yes". The only actions are Close and Copy. Source shows Copy puts all three lines on the clipboard. Security (WPA/WEP/Open) is never shown.
- **Consequence:**
  - To join the network, the user has to paste into GNOME Settings' password field, so they would need to trim the text by hand.
  - The password is shown in plain text by default to anyone looking at the screen.
  - The user can't tell which security type to pick.
  - The heading doesn't say what arrived.
- **Fix:** For the `Wifi` payload only (leave text and links as they are), replace the TextView with a boxed list:
  - `ActionRow` "Network": SSID as the subtitle, with a copy suffix button.
  - `ActionRow` "Security": e.g. "WPA/WPA2 Personal". Skip the password row when Open.
  - "Password": show it masked (●●●●), with a reveal toggle and a copy suffix.
  - "Hidden Network": Yes/No as the subtitle.
  - Heading: "Wi-Fi Network from {device}". Responses: "Close" and "Copy Password" (suggested). Show a toast "Password copied" when copied. Keep "no automatic connection".
- **Pass:** Copy Password puts only the password string on the clipboard. The password is masked until revealed. The security type is visible. No editable-looking text cursor appears.

### 2. The consent dialog doesn't say it's a Wi-Fi network [V]
- **Evidence:** In `consent.png` the body says "Text, a link, or Wi-Fi credentials". It's the generic fallback in `transfers.rs:425`.
- **Consequence:** The user gives consent without knowing what is coming. For credentials, that is the most important thing to know. Files get a specific description, but Wi-Fi does not.
- **Fix:** When `meta.payload` is already `Wifi` at consent time, use "Wi-Fi network “{ssid}”". If only the kind is known, use "Wi-Fi network details". Use "A link" or "A text message" when the kind is known. Keep the generic string only when the kind is really unknown. [H] I'm assuming the introduction frame exposes the kind or SSID before acceptance. Use whatever is actually available.
- **Pass:** Consent for the fixture reads "Wi-Fi network “Quickshare test network”" (or "Wi-Fi network details").

### 3. Activity shows "0 bytes" for Wi-Fi [V]
- **Evidence:** In `activity.png` the subtitle is "Complete · From Quickshare test peer · 0 bytes · 06:41".
- **Consequence:** "0 bytes" looks like the transfer failed or was empty.
- **Fix:** Leave out the size part for Wi-Fi, and for text/link when the size is 0 or meaningless: "Complete · From {device} · {time}".
- **Pass:** No Wi-Fi or text history row shows a size.

### 4. The Send page still frames the task as files/text while a Wi-Fi draft is active [V]
- **Evidence:** In `wifi-preview.png` the subtitle still says "Choose files or text, then select a device below." The drop zone stays the biggest element above the draft. The clear button on the draft row uses `edit-clear-symbolic` (the backspace-shaped glyph), which reads as "erase a character" more than "remove this item". On the narrow screen (`wifi-small.png`) the SSID and the subtitle each wrap to two lines.
- **Fix:**
  - Change the clear suffix to `window-close-symbolic` (tooltip "Remove Wi-Fi Details", already similar) and the `flat` + `circular` classes, the same as the file row's remove affordance. That file row uses a minus. Pick one glyph for "remove from selection" across both rows.
  - Shorten the subtitle to "Wi-Fi network" (the page already says to pick a device below). That lets it fit on one line at 360.
  - Optional: change the page subtitle to "Choose what to share, then select a device below."
- **Pass:** File and Wi-Fi rows use the same remove glyph. At 360 the Wi-Fi subtitle fits on one line.

### 5. Wi-Fi sharing can only be found in the hamburger menu [V]
- **Evidence:** In `wifi-preview.png` the drop zone offers only "Choose Files…" and "Share Text…". Wi-Fi only appears after you use the main menu.
- **Fix (low cost, no new layout):** Add a third flat button "Share Wi-Fi…" under the two pills, or put it as a menu item in a small `MenuButton` next to "Share Text…". Keep the main-menu entry. If you want to keep the drop zone simple, this is fine to defer.
- **Pass:** The Wi-Fi flow can be reached from the Send page without opening the main menu.

### 6. Form: Enter and default response behaviour [H]
- **Evidence (source):** `set_default_response("cancel")`, and the entry rows don't set `activates-default`.
- **Consequence:** After filling the password, Enter either does nothing or, if anything gets wired to the default response, cancels and throws away the input.
- **Fix:** Make "prepare" the default response and set `activates_default(true)` on the name and password rows. The response is already disabled while invalid, so Enter can't send bad data.
- **Pass:** With valid input, pressing Enter in the password field opens the preview. With invalid input, Enter does nothing.

## Not issues
- The hint text updates with the security type (Open/WEP/WPA). The password row is insensitive for Open. Both are fine.
- No automatic connection and no payload in history: correct.

## Limits
Not checked against real Android or Windows devices. Dark and high-contrast themes were not captured. The narrow form dialog (`wifi-form`, 310 wide) looked fine. The received dialog was not captured narrow.
