# Quickshare UI re-check (Claude)

Read-only re-review of 8 updated screenshots against `claude-review.md`. Source was read for context only.

## Verified fixed
- **#1 Recipient below the fold:** fixed at 820. "Nearby Devices", its spinner, the empty state and the privacy line all fit, in the empty state, with a file selected and with the composer open (`send`, `files`, `text`).
- **#2 Text sharing:** fixed. "Share Text…" is a pill next to "Choose Files…". The inline "Text or Link" composer has a clear follow-up hint.
- **#3 Activity completion:** fixed. The row shows the filename as its title, a green check, "Complete · From … · 50 B · 03:35" and "Open Folder". The progress bar and code are gone. Open Folder instead of Open is a reasonable choice given how filename collisions are handled.
- **#4 Preferences:** fixed. There is no Save button and no search. "Changes are saved as you make them." is shown. The folder row has a single chooser button. Automatic Port is a switch with the current port shown below it.
- **#5 Header:** fixed. The folder-open icon no longer looks like the Receive glyph. The tagline is gone. The switcher sits in the header at 820, and below 550sp it moves to a bottom bar with the title shown.
- **#6 to #8, #10:** Clear is hidden when nothing is selected, the spinner is in. Centred text is now centred. Receive shows a single countdown ("Visible for 10 more minutes") with edit and folder-open suffixes. The stray dash is gone. Metadata is darker and easy to read.

## Remaining issues (ranked, max 4)

### 1. At 360x640 the Send page still hides the recipient step [V]
- **Evidence:** In `small-send`, with one file selected, the view ends at the sample.txt row. "Nearby Devices" is fully off-screen. The 24px `page-title` wraps to two lines ("Send to a Nearby / Device"). The subtitle wraps too. Together they use about 150px. The stacked full-height pills and the drop-zone icon and caption add more.
- There is also a rendering artefact: small dark specks/tick marks just above the title, at about (165,75) and (290,75). They look like clipped glyph fragments or a scroll undershoot drawing over the title. They do not appear at 820.
- **Fix (in the existing 550sp breakpoint):** switch the title to `title-2` (or hide it, since the bottom bar already says "Send"). Hide the page subtitle. Hide the drop-zone icon and the "Or choose what to share" caption. Keep the vertical pill stack but cut drop-area padding to 12px. Find the speck source, for example by checking whether something overlaps the top 8px of the title.
- **Pass:** At 360x640 with one file selected, the "Nearby Devices" heading and spinner are visible without scrolling. There are no stray marks above the title.

### 2. Preferences shows "My computer" as selected text when it opens [V]
- **Evidence:** In `preferences`, "My computer" in the Device Name row has a grey selection highlight even though focus moved away (`page.grab_focus()`). Autofocus is fixed, but the entry keeps its select-all region, which then shows as an inactive selection.
- **Consequence:** It looks like the text is selected and about to be replaced. If the user tabs back into the field, the selection is still active.
- **Fix:** After building the row, clear the selection. For example, call `name.select_region(-1, -1)` / `set_position(-1)` after it maps (`connect_map` → idle), or defer `page.grab_focus()` until after the window has presented.
- **Pass:** Preferences opens with no highlighted text in any row.

### 3. Narrow Receive cuts the received path off at the end, hiding the folder that matters [V]
- **Evidence:** In `small-receive`, the subtitle reads "/tmp/nix-shell.hwv27Q/quickshare…". The meaningful last part ("received") is lost. Leaving `/tmp` un-abbreviated is correct.
- **Fix:** Ellipsize in the middle so the end stays visible. AdwActionRow's subtitle label uses end ellipsis, so do it in `display_path`. When the path is longer than about 40 characters, keep the first part and the last two parts, for example `/tmp/…/quickshare-ui.8ylcLC/received`. Keep the tooltip with the full path (already there). Apply the same thing to the Preferences Receive Folder row.
- **Pass:** At 360 wide the Received files subtitle ends with "/received".

### 4. Heading capitalization is inconsistent [V]
- **Evidence:** "Nearby Devices" (Send) and "This Computer" / "Background" / "Connection" (Preferences) use header capitalization. "This computer" (Receive) and "Your transfers" (Activity) use sentence case. The same group is spelled "This Computer" in Preferences and "This computer" on Receive.
- **Fix:** Following the HIG, group headings use header capitalization: "This Computer" on Receive (`window.rs:318`) and "Your Transfers" on Activity (`window.rs:373`). Leave the empty-state sentence at `window.rs:378` as it is.
- **Pass:** Every group heading uses header capitalization, and the same group has the same name everywhere.

## Notes (not counted)
- [H] In both 360 shots the header folder button has a visible focus ring, and at 820 the Receive visibility row is outlined in blue. This is probably keyboard focus left over from the capture script. If the app shows it on a normal launch with no key presses, set the initial focus to nothing (`window.set_focus(None::<&gtk::Widget>)`).
- The "03:35" timestamp has no date context. That is fine for today's items. Consider "Yesterday 03:35" or a date for older history later. Not for this pass.
