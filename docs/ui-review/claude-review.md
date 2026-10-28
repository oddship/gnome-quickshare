# GNOME Quickshare UI review (Claude)

Scope: six screenshots (Send, Receive, Activity at 820x820; Preferences at 520 wide; Activity at 390x800; incoming consent dialog).
Source read for context only: `src/window.rs`, `src/window/{preferences,transfers}.rs`, `data/style.css`.
Tags: **[V]** = confirmed visual finding from a screenshot. **[H]** = interaction or a11y hypothesis that needs a check.

## Top 5 (do these first)

### 1. Send: the recipient step is below the fold [V]
- **Evidence:** In `send.png` the hero (icon, "Share a little closer.", two-line tagline) uses about y=125 to y=305. The drop zone runs to y=535. "Nearby devices" starts at y=665, and its empty-state copy is cut at the bottom edge. The privacy line ("Direct transfers. No cloud. No account.") and any device rows cannot be seen at 820x820.
- **Consequence:** Step 2 of the main flow (pick a recipient) needs a scroll on a large window. When a phone shows up, the user may not notice it.
- **Fix:**
  - Remove the Send hero, or shrink it to one 48px icon plus one `title-2` line. The drop zone already says what to do. Keep the `hero-title` style only for empty states.
  - Cut drop-area padding from 24 to 18px and its inner spacing from 14 to 10. Cut the page `vertical(24)` spacing to 18.
  - Use an `AdwPreferencesGroup` titled "Nearby Devices" with a header-suffix `GtkSpinner` (see #6). Do not use the custom `section` + label box.
- **Pass:** At 820x820 with nothing selected, the "Nearby Devices" heading and the first device row, or the full empty state, are visible without scrolling. With one file selected, the first device row is still visible.

### 2. Text/link sharing is hard to find [V]
- **Evidence:** "Share text or a link" is a bare `GtkExpander` at y=624. It has no card, no icon, and normal body weight. Its chevron sits 4px left of the content's left edge (x≈117 vs 113), so it does not line up with "Nothing selected yet" or "Nearby devices". It reads as a footnote and not as one of the two ways to share.
- **Consequence:** Users who want to send a link will not find the feature. Files look like the only option.
- **Fix:** Put text on the same level as files inside the drop zone. Add a second pill button, "Share Text…", next to "Choose Files…" (a horizontal box, 12px spacing, centred). It opens an `AdwDialog` with a `GtkTextView` (or an `AdwEntryRow` for a single link) and an "Add" button. You could also reveal the text card inline in place of the expander. Change the drop-zone caption to "Or choose files or text to share". Remove the expander.
- **Pass:** Both ways to share are visible in the drop zone at the same visual weight. No hidden disclosure. Keyboard: Tab reaches "Share Text…" right after "Choose Files…".

### 3. Activity: a finished transfer gives no way to act on the result [V]
- **Evidence:** In `activity.png` and `narrow.png`, the finished row shows "From Quickshare test peer / Complete · 50 B of 50 B · Code 2421". A full blue progress bar sits *outside* the card, 8px below it, like a separate element. There is no filename ("sample.txt" shows in the consent dialog, but not here), no timestamp, no Open or Show in Files action, and no way to dismiss. The rest of the page (about 560px) is empty.
- **Consequence:** After a receive, the user does not know where the file went. They have to find the header folder button or a raw path on Receive. The finished bar looks like the transfer is still running or stuck.
- **Fix:**
  - On a terminal state, hide the progress bar (`progress.set_visible(false)`). While the transfer runs, put the bar inside the card (bottom of the row, or an `osd`-style bar in the row's content) so it does not float.
  - Title: the file name ("sample.txt", or "3 files"). Subtitle: "From Quickshare test peer · 50 B · Just now". Drop the code after completion. The code only matters before consent.
  - Inbound Finished: add a suffix `flat` button "Open" for a single file, plus a "Show in Files" icon button (`folder-open-symbolic`, tooltip "Show in Files"). Make the row activatable to open it.
  - Failed or Declined: use a `warning`/`error` styled state word and add a "Try Again" action for outbound.
  - Cancel button: keep it for active transfers. On terminal rows, use a "Remove from list" icon (or a page-level "Clear Finished").
- **Pass:** After a test receive, the finished row shows filename, sender, size and relative time. It has a working Open/Show in Files action and no progress bar. Running transfers show the bar inside the card.

### 4. Preferences: Save is not visible, and the dialog behaves like a form, not GNOME prefs [V]
- **Evidence:** In `preferences.png` (520x576) no Save button is visible. It sits under Connection and Privacy (source: `Save Preferences` in the last group). The header has a search button for a short page. "Device name" opens focused with its text selected (highlighted "My computer"). "Receive folder" is an editable entry with *both* a pencil and a folder button. The port uses the placeholder-title "Static TCP port (blank for automatic)". The group description says changes need a restart.
- **Consequence:** Users toggle "Start at login", close the window, and lose the change without knowing. A stray keypress on open overwrites the device name. Two ways to edit the folder makes people unsure which one counts.
- **Fix (GNOME HIG: preferences apply immediately):**
  - Remove the Save button. Persist each control on change: switches on `notify::active`, entry rows on `apply` (set `show-apply-button(true)` on Device name and Port), folder on chooser response. Validate per field and show an error by adding the `error` CSS class on the row plus a toast. Do not block other fields.
  - When a change needs a restart, show an `AdwBanner` at the top of the page ("Restart Quickshare to use the new name and port" with a "Restart" button), or restart the backend in place. Do not rely only on the static group description.
  - Do not autofocus the name entry. Focus the dialog itself, or call `grab_focus` on the close button.
  - Receive folder: use an `AdwActionRow`. Title "Receive Folder", subtitle a short path (`~/Downloads`). One suffix `folder-open-symbolic` button that opens `GtkFileDialog::select_folder`. No free-text entry, no pencil.
  - Port: an `AdwEntryRow` titled "Port" with input-purpose digits, or an `AdwSpinRow` with an "Automatic" switch row above it. Put the help text in the group description, not the title.
  - Turn off search (`search-enabled(false)`) for a single short page.
- **Pass:** Toggling any switch and closing the dialog with the X keeps the change after reopening. No Save button exists. Opening Preferences selects no text. The folder row has exactly one edit affordance.

### 5. Header bar: the tagline subtitle and an unclear download button [V]
- **Evidence:** Every main screenshot shows the title "Quickshare" with the subtitle "A little closer." The leftmost header button is a download arrow. It is the same glyph as the Receive tab, but it opens the received-files folder (tooltip "Open Received Files"). The copy says "Quickshare" in the title and "Quick Share" in the empty state.
- **Consequence:** The tagline uses header space and says nothing about state. The arrow looks like "Receive" or "Download now". The name changes from place to place.
- **Fix:**
  - Drop the header subtitle, or use it for live state ("Visible · 9 min left" / "Hidden"). That helps more than the tagline. Better still, put the `ViewSwitcher` in the header as the title widget (see #9) and remove the separate switcher row. That saves about 45px vertically.
  - Change the left button icon to `folder-open-symbolic` (or move "Open Received Files" into the main menu). It should not share a glyph with the Receive tab.
  - Copy rule: the app is "Quickshare". The phone feature is "Quick Share" (Android's name). Use each on purpose.
- **Pass:** No header element repeats a tab icon. The header subtitle is either removed or shows real visibility state.

## Further issues (ranked)

### 6. Send selection and discovery states show controls that do nothing [V]
- **Evidence:** "Clear" is visible and looks enabled when the label says "Nothing selected yet". "Searching" is plain dim text with no activity indicator. The empty state "Who's nearby?" with its Wi-Fi glyph shows at the same time as "Searching", so the two messages compete.
- **Fix:** Hide the whole selection header (label and Clear) until something is selected, or set Clear to insensitive. Once items exist, show "2 files · 3.4 MB" plus Clear. Replace "Searching" with a 16px `GtkSpinner` and the tooltip "Looking for nearby devices". After N devices are found, show "2 found". Show the empty state as an `AdwStatusPage` (`compact` class) only after about 5s with nothing found. Copy: "No devices found yet" plus the existing two hint lines.
- **Pass:** With nothing selected, no Clear button is visible. While discovery runs, an animated spinner is visible.

### 7. Centred hero text is left-aligned inside its box [V]
- **Evidence:** In `send.png` both tagline lines start at x≈276 ("Files, links…" / "Straight to…"). They are ragged on the right under a centred heading. `receive.png` has the same problem (x≈265), and so do the Receive "You're always in control" lines (x≈229) and the Send empty-state hints (x≈211). These are multi-line `GtkLabel`s with default `justify=left`.
- **Fix:** Call `set_justify(gtk::Justification::Center)` on every centred multi-line label (or in the `label()` helper when it is used in centred contexts). Better still, replace the hand-built empty states with `AdwStatusPage` (class `compact`), which already centres and wraps.
- **Pass:** Each line of every centred block is centred on the same axis as its heading.

### 8. Receive: repeated copy, raw path, a misleading chevron, and status below the fold [V]
- **Evidence:** "10 minutes" appears twice ("You're visible for 10 minutes…" and "Visibility switches off after 10 minutes."). The headline "Make room for something good." plus tagline uses about 190px. "Received files" shows `/tmp/nix-shell.HjiH2Y/quickshare-ui…/received` in full. Its suffix is a `go-next` chevron, which in Adwaita means "go to a subpage", but here it opens an external file manager. "My computer" looks like it could be tapped but cannot be edited here. A small stray dash/dot shows at about (211,658) between the card and the lock icon. The `status-pill` ("Starting Quick Share…") is in source but is below the fold.
- **Fix:**
  - Shrink the hero as in #1. Make the visibility row the first thing on the page.
  - Put the countdown on the row itself. Subtitle when on: "Visible for 9 more minutes". When off: "Hidden. Turn on to receive." Remove the separate line below the card and the duplicate in the "always in control" block. Keep that block to "Check the matching code before accepting. Nothing is accepted automatically."
  - Shorten the path: replace `$HOME` with `~` and ellipsize in the middle (`set_subtitle_lines(1)`; the long path should not wrap). Change the suffix to `folder-open-symbolic` and make the row activatable.
  - "My computer" row: add a `document-edit-symbolic` suffix (or make it activatable) that opens the name field in Preferences.
  - Move the service status ("Starting…", "Connect to a local network") into an `AdwBanner` at the top of the page. It only shows when the service is not ready. Find and remove the stray dash widget (probably an empty label or separator between `name_group` and `consent_info`).
  - Switch row: check the switch's sensitivity. Source creates it `sensitive(false)` until the backend starts. It needs a tooltip or subtitle explaining why while it is disabled. [H]
- **Pass:** "10 minutes" appears once and counts down. The path fits on one line with `~`. No chevron opens an external app. No stray dot.

### 9. Narrow layout: the tab row crowds the header and does not adapt [V]/[H]
- **Evidence:** In `narrow.png` (390 wide), the Wide-policy switcher fills nearly the full width (Send pill at x≈18, Activity pill ends at x≈378). The header still holds the folder button, title + tagline, menu and three window controls. Only Activity was captured narrow, so it is not confirmed whether Send/Receive content wraps well at 390. [H]
- **Fix:** Use the standard adaptive pattern. Put `AdwViewSwitcher` (policy Wide) as the header title widget. Add an `AdwViewSwitcherBar` at the bottom. Add an `AdwBreakpoint` at `max-width: 550sp` that hides the header switcher and reveals the bar. Remove `switcher_box`. Also check that the hero titles (30px, 800 weight) wrap and do not ellipsize at 360px.
- **Pass:** At 390 wide the tabs are in a bottom bar, and the header shows only the title, the folder button, the menu and window controls. At 820 the tabs are in the header. Send and Receive at 360x640 have no horizontal clipping.

### 10. Low-contrast small text [H]
- **Evidence:** Many small grey strings stack `dim-label` (about 55% opacity) on 11 to 12px captions: "Searching", "Nothing selected yet", "Or choose what you'd like to share", the row subtitles, "Complete · 50 B…". The `.hero-subtitle` uses `opacity: 0.65`. By eye these look about #8a8a8a on #fafafa, below 4.5:1.
- **Fix:** Do not use `dim-label` for text a user must read to act ("Nothing selected yet", status words). Use the default fg colour, or the `caption` class without `dim-label`. Replace `.hero-subtitle { opacity: .65 }` with Adwaita's `dimmed`/`dim-label` only on text at body size or larger. Use `color: @window_fg_color` with `opacity: .8` for the tagline if you keep it. Check with the high-contrast setting and a contrast picker.
- **Pass:** All task-relevant text measures at least 4.5:1 in the light theme. High-contrast mode shows no faded text.

## Consent dialog (`consent.png`)
Strong overall. The sender is in the heading, file count/size/name are shown, the instruction is clear, the code is big (36px with tabular numbers), and Decline/Accept are equal width with Accept suggested. Small items:
- [H] Make Decline the default and close response, so Escape and a stray Enter do not accept. Keep Accept as `suggested`, but only make it the default after focus moves on purpose.
- [H] The letter-spaced "2 4 2 1" may be read digit by digit. That is fine, but set an accessible label "Code 2421" on it.
- [V] Put the instruction right above the code, and make it shorter: "Accept only if this code matches the other device". Today the code sits below a two-line wrapped sentence that ends with an orphan "device." Allow 40px+ width or shorter copy so the line does not break into an orphan.
- For several files, list at most 3 names, then "and 4 more".

## Strengths
- Native Adwaita all the way through (boxed lists, `AdwActionRow`, pills, toasts, a `MessageDialog` for consent). Theme, dark mode and focus rings should just work.
- The consent and matching code flow is clear and puts trust first. "Nothing is accepted automatically" and the account-free privacy line are good, honest copy.
- The 660px clamp keeps reading width comfortable at 820 wide.
- Three tabs (Send, Receive, Activity) map cleanly to the user's tasks.

## Limits of this review
- No real phone, so device rows, multi-device lists, active progress, failures and queued transfers were not seen. Their styling is judged from source only.
- Send and Receive were captured only at 820. Only Activity was captured narrow. Preferences below "Connection" was not visible.
- Only the light theme. Contrast numbers are by eye, not measured.
