# Visual hunt: what the eye found in the real window

This is the evidence behind the visual bug reports. Nothing here fixes
anything; it drives the real window, screenshots each state, and records the
number that makes the state wrong, so a fix lane can assert it.

Run it:

    ./applab/probes/run-probe.sh visual-hunt

It is a probe, not a spec: `applab/specs/` is the regression suite and stays
green, while a probe is meant to *hold* a bug. Screenshots land in
`$LAB_SCREENS` (default `/tmp/gitgud-visualhunt`); the copies under
`applab/probes/screens/` are the frames the reports were written from, plus a
zoomed crop of each.

The fixture is built by the probe itself: a long Turkish path
(`src/ozellikler/kullanici-yonetimi/bilesenler/kullanici-oturum-acma-formu-bileseni.tsx`)
in a real git merge conflict and in a commit, plus one local change, so all
three panels get exactly the content that breaks them. Window sizes are the
ones the app really has: 800x500 is its own `minWidth`/`minHeight`, 1000x700
and 1280x800 are ordinary desktop sizes.

## 1. ConflictView: the conflicted file's name collapses to "src..."

Screens: `screens/conflict-800x500.png`, `screens/conflict-header-zoom.png`,
`screens/conflict-1280x800.png`.

Eye: at 800x500 the sticky header over a conflict shows "src..." and nothing
else of the path; the three resolution buttons own the first line, and the
"mark as resolved" button drops to a second line on its own. At 1000x700 the
name gets 55px ("src/ozellikler/ku..."), at 1280x800 335px.

Number at 800x500 (`section.isolate .sticky`):

    rowW 480, rowH 67, flexWrap "wrap"
    name cell: clientWidth 33, scrollWidth 635, hidden 602px
    buttons on the first line: 199 + 207 + 170 = 576px in a 480px row

At 1000x700 the name is 55px, at 1280x800 it is 335px: the row grows, the
buttons do not, so the name only recovers when the pane is wide.

Suspect: `src/components/ConflictView.tsx:78` (the row) and `:79` (the name,
`min-w-0 flex-1 truncate`). A `flex-wrap` row whose first item is `flex-1` with
`min-w-0` will give that item one character before it wraps the buttons.

Expected: the path is visible, or at least long enough to be identifiable, at
every width the window allows; the buttons wrap instead of the name shrinking
to a stub.

Smallest fix: make the name take its own line (a full-width row above the
buttons, e.g. `basis-full` on the name or wrap the buttons in their own
`flex flex-wrap` group), so the name keeps the row width and truncates at the
pane edge, not at 33px. Gate:
`document.querySelector('section.isolate .sticky span.truncate').clientWidth >= 0.6 * rowWidth`
at 800x500, and the row still inside 2 lines.

Severity: during a merge the header is the only place that names the file being
resolved; "src..." is not a name. Tooltip has the full path on hover, so it is
recoverable, not fatal.

## 2. FileHeader: the path in a commit's diff header is 12px wide

Screens: `screens/commit-800x500.png`, `screens/commit-header-zoom.png`,
`screens/commit-1280x800.png`.

Eye: in a commit, the bar above the diff reads "s. +3" plus five icon buttons.
The whole path is one letter and a period. The file list on its left is also
cut ("kullanici-otur... +3"), so the commit's right pane names nothing.

Number at 800x500:

    sidebar 440, section (pane) 360, file-list column clamped to 160 (its min)
    header row 200px wide; its cells:
      path div 12px (flex-shrink 1)
      line counts 14, then 20 + 20 + 22 + 20 + 20 of icon buttons (shrink 0)
    path scrollWidth 525, clientWidth 12, hidden 513px

At 1280x800 the same row is 544px and the path gets 356px, so this is purely
the narrow pane: the file-list column keeps 160 of the 360, and five
`shrink-0` controls take 102 of the remaining 200.

Suspect: `src/components/FileHeader.tsx:76` (the path, `min-w-0 flex-1
truncate select-text`) and `src/components/PatchView.tsx:52-57` (the file
column: `initial 288, min 160, keepForRest 240`).

Expected: at the app's minimum window the path shows enough to identify the
file, or the header offers the path through a control that costs nothing when
the pane is narrow.

Smallest fix: when the header row is below some width, drop the trailing icon
group to what fits (the "ignore whitespace" and "side by side" toggles are also
in the file's context menu and in the command palette) or wrap the path onto
its own line. Gate: path `clientWidth >= 80` at 800x500 with the file list at
its 160px minimum, and `>= 120` at 1280x800.

Severity: the diff is legible, the file it belongs to is not, and the tooltip
needs a hover. Same class as 1 but in the much more used commit view.

## 3. ChangesPanel at its minimum width: the commit button and the section titles lose their text

Screens: `screens/changes-sidebar220.png`, `screens/changes-sidebar220-zoom.png`,
`screens/changes-sidebar320.png`.

Eye: with the sidebar dragged to its own minimum (220px, which the resize
handle allows) the primary button reads "1 dosy..." and the staged section
header reads "HAZIRLA...". The "Tümünü hazırlıktan çıkar" bulk action next to
it is complete. At the default 320px everything reads fully.

Number at sidebar 220 (`aside` clientWidth 219):

    stage header row: title span clientWidth 46, scrollWidth 126
    same row's bulk button: 141px, flex-shrink 0
    commit button: clientWidth 66, scrollWidth 127 (label "1 dosyayı commit et")

At 320: title 126 of 126, commit button 166 of 166.

Suspect: `src/components/ChangesPanel.tsx:572-583` (header: title
`min-w-0 truncate`, bulk `shrink-0`) and `:493` (commit button `min-w-0
flex-1 truncate` next to two `shrink-0` buttons).

Expected: at the width the app itself permits, the primary action and the
section title are readable.

Smallest fix: give the bulk action and the two button row labels a
`hidden`/short form under a container query, or let the bulk action wrap to its
own line (it is `justify-between`, so `flex-wrap` plus `basis-full` on the
title fixes the header with one class). Gate:
`commitBtn.scrollWidth <= commitBtn.clientWidth` and
`titleSpan.scrollWidth <= titleSpan.clientWidth` at sidebar 220; both already
hold at 320, so the fix must not change 320.

Severity: the app's own drag handle lets the user reach a width where the
commit button's label is gone; the button still shows its color and the Ctrl
tooltip, so the action is findable but unreadable.

## 4. Also confirmed, weaker: diff line numbers sit below the theme's own contrast floor

Screens: `screens/diff-gutter-light.png`, `screens/diff-gutter-light-zoom.png`,
`screens/diff-gutter-dark.png`, `screens/diff-gutter-dark-zoom.png`.

Eye: the line number "1" on an added line is grey on mint in the light theme
and barely lifts off the row; the same number in the dark theme is thin but
readable. Nothing is clipped; this is purely contrast.

Number (canvas composite of the computed colour over the row's real backdrop,
so alpha and the oklab color-mix are both honoured):

    light theme, added row: painted rgb(144, 153, 160) on rgb(230, 246, 234)
      contrast 2.58;  the muted token itself on the same row is 4.31
      on the plain panel backdrop the same colour is 2.76
    dark theme, added row: 3.18; muted token 5.03

The app asserts its own floor in `src/lib/themes.test.ts:41`
(`contrast(c.muted, bg) > 3`). The gutter is that token at 70% alpha
(`src/components/DiffView.tsx:225`, `text-muted/70`), so it cannot meet a floor
it is not tested against.

Expected: line numbers, which are read to scan a diff, meet the same 3:1 floor
every other muted text does.

Smallest fix: `text-muted/70` becomes `text-muted/85` (or plain `text-muted`),
and the floor gets a test of its own over the diff row backgrounds. Gate:
canvas-composited contrast of the gutter on `added-bg` and on `bg` is `>= 3.0`
in every theme in `src/themes.css`.

Severity: low. It costs a little legibility on every diff line, for everyone,
in every light theme; nobody reports it, everybody reads slightly worse.

## Looked at and clean (no bug)

- start screen at 800x500, 1000x700, 1280x800: centered, no sideways overflow
  (`documentElement.scrollWidth == innerWidth` at all three).
- start screen with 10 long Turkish recents at 800x500: scrolls
  (`scrollHeight 911`, `clientHeight 500`), rows truncate with `title`, no
  overlap.
- start screen with a recent path that is not a repository: error line renders
  in `text-removed`, nothing overflows.
- toolbar at 800x500: every label ellipsizes, but each button carries the full
  text in `title`, and the layout does not collide. Deliberate mode, not a bug.
- branch menu with 40-character Turkish branch names: 318 of 318 px, rows
  truncate with the full name in `title`.
- history with a merge commit, tags and long ref badges: badges cap at
  `max-w-24` and truncate, the merge graph draws both lanes.
- stashes with a 100-character Turkish message: one line, truncated, `title`
  has the whole message.
- command palette (576x350 at 800x500): box fits, list scrolls (308 of 1184).
- context menu opened at the bottom-right corner: clamped inside the window
  (bottom 496 of 500).
- appearance dialog at 800x500: fits, body scrolls.
- empty repository (git init, no commits): "No local changes", commit button
  disabled, no overflow.
- dark theme on the same screens: layout identical, no new clipping.
- inline diff wider than its pane: the pane is its own scroller and scrolls, by
  design (`table-fixed` only in split mode, where lines wrap).
