// Visual hunt: the three confirmed visual bugs, reproduced against the real
// window on a repository shaped like a real one (long Turkish paths, a real
// merge conflict, local changes).
//
// Each section prints the number that makes it a bug and saves a screenshot
// next to it, so the fix lane has both the eye and the assertion:
//
//   1  ConflictView : the conflicted file's name is clipped to a few letters
//   2  FileHeader   : the path in a commit's diff header collapses to "s…"
//   3  ChangesPanel : at its minimum width the commit button and the stage
//                     section's title lose their text
//
// Run with applab/probes/run-probe.sh visual-hunt. It is a probe, not a spec:
// it asserts nothing, it measures. Screenshots go to $LAB_SCREENS (default
// /tmp/gitgud-visualhunt).
import { execFileSync } from 'node:child_process'
import { mkdirSync, writeFileSync } from 'node:fs'
import { launch } from '../../applab/lab.mjs'

const repo = process.env.LAB_REPO
const OUT = process.env.LAB_SCREENS ?? '/tmp/gitgud-visualhunt'
mkdirSync(OUT, { recursive: true })

const git = (...args) =>
  execFileSync('git', ['-C', repo, ...args], { encoding: 'utf8' }).trim()

const write = (p, body) => {
  mkdirSync(p.split('/').slice(0, -1).join('/'), { recursive: true })
  writeFileSync(p, body)
}

/** A long path, so nothing can be blamed on a short name. */
const CONFLICT =
  'src/ozellikler/kullanici-yonetimi/bilesenler/kullanici-oturum-acma-formu-bileseni.tsx'

/**
 * A repository with a long Turkish path in a conflict and in a commit, and
 * a local change, so all three panels have exactly the content that breaks
 * them. The conflict is a real one, made by git, not a fixture file.
 */
function shapeRepo() {
  write(
    `${repo}/${CONFLICT}`,
    'export const form = 1\nexport const dogrulama = 1\nexport const akis = 1\n',
  )
  git('add', '-A')
  git('-c', 'user.name=App Lab', 'commit', '-qm', 'add the long path component')

  git('checkout', '-q', '-b', 'conflict/long-path')
  write(
    `${repo}/${CONFLICT}`,
    'export const form = 2\nexport const dogrulama = 1\nexport const akis = 1\n',
  )
  git('add', '-A')
  git('-c', 'user.name=App Lab', 'commit', '-qm', 'the other side')

  git('checkout', '-q', 'main')
  write(
    `${repo}/${CONFLICT}`,
    'export const form = 3\nexport const dogrulama = 1\nexport const akis = 1\n',
  )
  git('add', '-A')
  git('-c', 'user.name=App Lab', 'commit', '-qm', 'this side')
  try {
    git('-c', 'user.name=App Lab', 'merge', 'conflict/long-path')
  } catch {
    // The conflict is the point.
  }
}

async function shot(browser, name) {
  await browser.pause(400)
  await browser.saveScreenshot(`${OUT}/${name}.png`)
  console.log(`  shot ${OUT}/${name}.png`)
}

/** Clicks the first element matching `selector` whose text has `needle`. */
async function click(browser, selector, needle) {
  const ok = await browser.execute(
    (sel, want) => {
      const els = [...document.querySelectorAll(sel)]
      const el = want ? els.find((e) => (e.innerText || '').includes(want)) : els[0]
      if (!el) return false
      el.click()
      return true
    },
    selector,
    needle ?? null,
  )
  if (!ok) throw new Error(`nothing to click: ${selector} ${needle ?? ''}`)
  await browser.pause(700)
}

/** Opens the fixture at `width`x`height`, in Turkish, overriding prefs. */
async function open(browser, width, height, prefs = {}) {
  await browser.setWindowSize(width, height)
  await browser.execute(
    (path, kv) => {
      localStorage.setItem('gitgud.locale', 'tr')
      localStorage.setItem(
        'gitgud.recentRepos',
        JSON.stringify([{ path, name: 'fixture' }]),
      )
      for (const [k, v] of Object.entries(kv)) localStorage.setItem(k, v)
    },
    repo,
    prefs,
  )
  await browser.refresh()
  await browser.pause(2200)
}

/**
 * Widths of every truncated text box in the right-hand pane, with the full
 * text in its title. `hidden` is what the eye loses.
 */
const TRUNCATED = () =>
  [...document.querySelectorAll('section.isolate span.truncate, section.isolate div.truncate')]
    .map((el) => {
      const r = el.getBoundingClientRect()
      return {
        cls: String(el.className).slice(0, 50),
        txt: (el.innerText || '').trim().slice(0, 60),
        w: Math.round(r.width),
        cw: el.clientWidth,
        sw: el.scrollWidth,
        hidden: el.scrollWidth - el.clientWidth,
        title: el.title || null,
      }
    })
    .filter((e) => e.txt && e.cw > 0)

/**
 * The contrast of a diff's line-number gutter as it is actually painted.
 *
 * WebKit answers `getComputedStyle().color` for a color-mix in oklab, so the
 * colour is not parsed by hand: a 1x1 canvas is filled with the backdrop the
 * cell really sits on and then with the computed colour, and the composite is
 * read back as sRGB pixels. That is the number a reader's eye gets.
 *
 * Reports every kind of row separately (context, added, removed, meta, hunk),
 * against that row's own background, and next to the same colour with the /70
 * alpha removed: the app's theme test gives muted a 3:1 floor.
 */
const GUTTER = () => {
  const paint = (backdrop, color) => {
    const c = document.createElement('canvas')
    c.width = 1
    c.height = 1
    const ctx = c.getContext('2d')
    ctx.fillStyle = backdrop
    ctx.fillRect(0, 0, 1, 1)
    ctx.fillStyle = color
    ctx.fillRect(0, 0, 1, 1)
    return [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3)
  }
  const rgbOf = (color) => paint('rgba(0,0,0,0)', color)
  const lum = ([r, g, b]) => {
    const f = (v) => {
      v /= 255
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4
    }
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
  }
  const ratio = (a, b) => {
    const la = lum(a)
    const lb = lum(b)
    return (
      Math.round(((Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05)) * 100) /
      100
    )
  }
  const backdrop = (el) => {
    for (let node = el; node; node = node.parentElement) {
      const bg = getComputedStyle(node).backgroundColor
      const m = bg.match(/[\d.]+/g)
      if (m && (m.length < 4 || Number(m[3]) > 0.9)) return bg
    }
    return 'rgb(255, 255, 255)'
  }
  const muted = getComputedStyle(document.documentElement)
    .getPropertyValue('--app-muted')
    .trim()
  const rows = [...document.querySelectorAll('section.isolate table tr')]
  const seen = []
  for (const tr of rows) {
    // The line-number cells are the first two of a code row; meta rows leave
    // them empty and put their text in the last cell.
    const tds = [...tr.children].slice(0, 2)
    const td = tds.find((c) => (c.innerText || '').trim())
    if (!td) continue
    const number = (td.innerText || '').trim()
    if (!number) continue
    const bg = backdrop(td)
    const fg = getComputedStyle(td).color
    const painted = paint(bg, fg)
    const solid = paint(bg, muted)
    const entry = {
      number,
      row: (tr.innerText || '').slice(0, 30),
      background: bg,
      color: fg,
      painted: `rgb(${painted.join(', ')})`,
      contrast: ratio(painted, rgbOf(bg)),
      mutedContrast: ratio(solid, rgbOf(bg)),
    }
    if (!seen.some((s) => s.background === entry.background)) seen.push(entry)
  }
  return {
    theme: document.documentElement.dataset.theme,
    counts: {
      tables: document.querySelectorAll('section.isolate table').length,
      trs: rows.length,
      tds: document.querySelectorAll('section.isolate table td').length,
      firstTexts: rows.slice(0, 4).map((tr) => [...tr.children].map((td) => td.textContent.trim())),
    },
    rows: seen,
  }
}

const browser = await launch()
try {
  shapeRepo()

  // ---- 1. ConflictView: the conflicted file's name ---------------------
  // src/components/ConflictView.tsx:78-81 -- a flex-wrap row whose three
  // buttons take the first line, and a name with min-w-0 flex-1 collapses to
  // whatever is left, which at 800x500 is one letter wide.
  console.log('\n1. ConflictView sticky header')
  for (const [w, h] of [
    [800, 500],
    [1280, 800],
  ]) {
    await open(browser, w, h)
    await click(browser, 'aside ul > li', 'kullanici-oturum-acma-formu')
    await shot(browser, `conflict-${w}x${h}`)
    const header = await browser.execute(() => {
      const row = document.querySelector('section.isolate .sticky')
      const name = row.querySelector('span.truncate')
      const r = row.getBoundingClientRect()
      const b = name.getBoundingClientRect()
      return {
        rowW: Math.round(r.width),
        rowH: Math.round(r.height),
        wrap: getComputedStyle(row).flexWrap,
        nameW: Math.round(b.width),
        nameCW: name.clientWidth,
        nameSW: name.scrollWidth,
        hidden: name.scrollWidth - name.clientWidth,
        buttons: [...row.querySelectorAll('button')].map((el) =>
          Math.round(el.getBoundingClientRect().width),
        ),
      }
    })
    console.log(`  ${w}x${h}: ${JSON.stringify(header)}`)
  }

  // ---- 2. PatchView's FileHeader: the path of a commit's file ----------
  // src/components/FileHeader.tsx:73-80 -- the path is `min-w-0 flex-1` and
  // every trailing icon button is `shrink-0`; when the pane is narrow the
  // path is last in line for room. src/components/PatchView.tsx:52-57 keeps
  // 240px for the diff and the file list clamps to its 160px minimum.
  try {
    git('merge', '--abort')
  } catch {
    // Not in a merge any more.
  }
  console.log('\n2. FileHeader path in the commit view')
  for (const [w, h] of [
    [800, 500],
    [1280, 800],
  ]) {
    await open(browser, w, h)
    await click(browser, 'aside > div > button', 'Geçmiş')
    await click(browser, 'aside ul > li', 'add the long path component')
    await shot(browser, `commit-${w}x${h}`)
    const boxes = await browser.execute(() => {
      const fileList = [...document.querySelectorAll('section.isolate div')].find(
        (el) => el.style.width && el.querySelector('ul'),
      )
      // The header row: its path cell plus every trailing control, so the fix
      // lane can see where the room goes.
      const pathCell = document.querySelector('section.isolate div.truncate.select-text')
      const headerRow = pathCell?.parentElement
      return {
        sidebarW: Math.round(
          document.querySelector('aside').getBoundingClientRect().width,
        ),
        sectionW: Math.round(
          document.querySelector('section.isolate').getBoundingClientRect().width,
        ),
        fileListW: fileList
          ? Math.round(fileList.getBoundingClientRect().width)
          : null,
        headerRowW: headerRow
          ? Math.round(headerRow.getBoundingClientRect().width)
          : null,
        headerCells: headerRow
          ? [...headerRow.children].map((el) => ({
              tag: el.tagName.toLowerCase(),
              txt: (el.innerText || '').trim().slice(0, 12),
              w: Math.round(el.getBoundingClientRect().width),
              shrink: getComputedStyle(el).flexShrink,
            }))
          : null,
        truncated: [...document.querySelectorAll('section.isolate div.truncate')].map(
          (el) => ({
            txt: (el.innerText || '').trim().slice(0, 50),
            cw: el.clientWidth,
            sw: el.scrollWidth,
            hidden: el.scrollWidth - el.clientWidth,
          }),
        ),
      }
    })
    console.log(`  ${w}x${h}: ${JSON.stringify(boxes)}`)
  }

  // ---- 3. ChangesPanel at the sidebar's minimum width ------------------
  // src/components/ChangesPanel.tsx:488-497 -- the commit button is
  // `min-w-0 flex-1 truncate` next to two `shrink-0` buttons; at the sidebar's
  // 220px minimum it is 66px wide for a 127px label.
  // src/components/ChangesPanel.tsx:572-583 -- the section title is
  // `min-w-0 truncate` beside a `shrink-0` bulk action that never gives way
  // and takes 141 of the 219px row.
  write(`${repo}/kalan.ts`, 'export const kalan = 1\n')
  git('add', '-A')
  console.log('\n3. ChangesPanel at the sidebar minimum (220px)')
  for (const sidebar of ['220', '320']) {
    await open(browser, 800, 500, { 'gitgud.width.sidebar': sidebar })
    await shot(browser, `changes-sidebar${sidebar}`)
    const panel = await browser.execute(() => {
      const rows = [...document.querySelectorAll('aside section > div')].map(
        (el) => {
          const title = el.querySelector('span')
          const bulk = el.querySelector('button')
          return {
            title: (title?.innerText || '').trim(),
            titleCW: title?.clientWidth,
            titleSW: title?.scrollWidth,
            bulkW: bulk ? Math.round(bulk.getBoundingClientRect().width) : null,
          }
        },
      )
      const commit = [...document.querySelectorAll('aside button')].find((b) =>
        b.className.includes('bg-accent'),
      )
      return {
        asideW: document.querySelector('aside').getBoundingClientRect().width,
        sections: rows,
        commit: commit
          ? {
              txt: (commit.innerText || '').trim(),
              cw: commit.clientWidth,
              sw: commit.scrollWidth,
            }
          : null,
      }
    })
    console.log(`  sidebar ${sidebar}: ${JSON.stringify(panel)}`)
  }

  // ---- 4. the diff's line numbers, against the theme's own floor -------
  // src/components/DiffView.tsx:225 -- the gutter is `text-muted/70`, i.e. the
  // muted token at 70% alpha, which composites below the app's own 3:1 floor.
  console.log('\n4. diff gutter contrast')
  await open(browser, 800, 500, { 'gitgud.width.sidebar': '320' })
  await click(browser, 'aside ul > li', 'kalan')
  await shot(browser, 'diff-gutter-light')
  console.log(`  light: ${JSON.stringify(await browser.execute(GUTTER))}`)
  // And in the dark theme, where muted is already the faint one.
  await browser.execute(() => localStorage.setItem('gitgud.theme', 'dark'))
  await browser.refresh()
  await browser.pause(2200)
  await click(browser, 'aside ul > li', 'kalan')
  await shot(browser, 'diff-gutter-dark')
  console.log(`  dark:  ${JSON.stringify(await browser.execute(GUTTER))}`)
} finally {
  await browser.deleteSession()
}
