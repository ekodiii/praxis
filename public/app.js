// ── IPC bridge ──────────────────────────────────────────────────────────
async function invoke(cmd, args) {
  return window.__TAURI_INTERNALS__.invoke(cmd, args);
}

// ── State ────────────────────────────────────────────────────────────────
let course = null;
let progress = {};
let activeChapterId = null;
let currentProjectFolder = null;
let testRunning = false;

// ── DOM refs ─────────────────────────────────────────────────────────────
const noCourseScreen = document.getElementById('no-course-screen');
const loadCourseBtn = document.getElementById('load-course-btn');
const noCourseBuiltin = document.getElementById('no-course-bundled');
const menuScreen = document.getElementById('menu-screen');
const menuCards = document.getElementById('menu-cards');
const menuShowAllBtn = document.getElementById('menu-show-all-btn');
const menuAllCourses = document.getElementById('menu-all-courses');
const menuLoadCourseBtn = document.getElementById('menu-load-course-btn');
const menuAppSettingsBtn = document.getElementById('menu-app-settings-btn');
const crumbPraxis = document.getElementById('crumb-praxis');
const crumbSep1 = document.getElementById('crumb-sep-1');
const crumbCourse = document.getElementById('crumb-course');
const crumbSep2 = document.getElementById('crumb-sep-2');
const crumbChapterEl = document.getElementById('crumb-chapter');
const openTerminalBtn = document.getElementById('open-terminal-btn');
const chapterList = document.getElementById('chapter-list');
const progressBarAscii = document.getElementById('progress-bar-ascii');
const courseOverview = document.getElementById('course-overview');
const overviewTitle = document.getElementById('overview-title');
const overviewDesc = document.getElementById('overview-description');
const overviewTech = document.getElementById('overview-tech');
const overviewChList = document.getElementById('overview-chapter-list');
const overviewSelFolderBtn = document.getElementById('overview-select-folder-btn');
const overviewFolderPath = document.getElementById('overview-folder-path');
const overviewEnvStatus = document.getElementById('overview-env-status');
const overviewEnvRows = document.getElementById('overview-env-rows');
const overviewEnvActions = document.getElementById('overview-env-actions');
const overviewEnvOutput = document.getElementById('overview-env-output');
const overviewLoadError = document.getElementById('overview-load-error');
const contentEmpty = document.getElementById('content-empty');
const contentBody = document.getElementById('content-body');
const runTestsBtn = document.getElementById('run-tests-btn');
const testResults = document.getElementById('test-results');
const testRows = document.getElementById('test-rows');
const testSummary = document.getElementById('test-summary');
const nextChapterBtn = document.getElementById('next-chapter-btn');
const testRunningIndicator = document.getElementById('test-running-indicator');
const serverCrashBanner = document.getElementById('server-crash-banner');
const infoNextBtn = document.getElementById('info-next-btn');

// ── Marked.js config ─────────────────────────────────────────────────────
marked.setOptions({
  highlight: (code, lang) => {
    if (lang && hljs.getLanguage(lang)) {
      return hljs.highlight(code, { language: lang }).value;
    }
    return hljs.highlightAuto(code).value;
  },
  breaks: true,
});

// ── Screen management ─────────────────────────────────────────────────────
// Screens: 'no-course' | 'menu' | 'overview' | 'course'

function showScreen(screen) {
  // no-course splash (no bundled course at all)
  noCourseScreen.classList.toggle('visible', screen === 'no-course');

  // main menu
  menuScreen.classList.toggle('visible', screen === 'menu');

  // sidebar: only in 'course' mode
  const inCourse = screen === 'course';
  document.getElementById('app').classList.toggle('sidebar-hidden', !inCourse);

  // topbar breadcrumb
  const hasCourse = !!course;
  crumbSep1.style.display = hasCourse ? '' : 'none';
  crumbCourse.style.display = hasCourse ? '' : 'none';
  if (hasCourse) crumbCourse.textContent = course.title;

  const hasChapter = inCourse && !!activeChapterId;
  crumbSep2.style.display = hasChapter ? '' : 'none';
  crumbChapterEl.style.display = hasChapter ? '' : 'none';

  // topbar right buttons
  openTerminalBtn.classList.toggle('visible', inCourse && !!currentProjectFolder);

  // main area sub-screens (inside #main)
  courseOverview.classList.toggle('visible', screen === 'overview');
  contentEmpty.style.display = (screen === 'course' && !activeChapterId) ? '' : 'none';
  contentBody.style.display = (screen === 'course' && !!activeChapterId) ? 'block' : 'none';
  testResults.classList.toggle('visible', false);

  // hide floating chapter-advance buttons when not in course view
  if (!inCourse) {
    nextChapterBtn.classList.remove('visible');
    infoNextBtn.classList.remove('visible');
  }

  // content header (breadcrumb bar + run tests) only in course mode
  document.getElementById('content-header').style.display = inCourse ? '' : 'none';
}

// ── Progress helpers ──────────────────────────────────────────────────────

async function loadProgress() {
  if (!course) return;
  try {
    const raw = await invoke('get_progress', { courseId: course.id });
    progress = raw || {};
  } catch (_) {
    progress = {};
  }
}

async function saveProgress() {
  if (!course) return;
  try {
    await invoke('save_progress', { courseId: course.id, chapters: progress });
  } catch (e) {
    console.error('save_progress failed:', e);
  }
}

// ── Chapter status logic ─────────────────────────────────────────────────
function getChapterStatus(chapter) {
  const saved = progress[chapter.id];
  if (saved && saved.status === 'complete') return 'complete';
  if (saved && saved.status === 'in_progress') return 'in_progress';
  if (!chapter.depends_on) return 'available';
  const depStatus = progress[chapter.depends_on];
  if (depStatus && depStatus.status === 'complete') return 'available';
  return 'locked';
}

function statusIcon(status, isInfo) {
  if (isInfo) return '[i]';
  return { complete: '✓', in_progress: '●', available: '○', locked: 'x' }[status] || '○';
}

function statusClass(status) {
  return { complete: 'complete', in_progress: 'active', available: 'available', locked: 'locked' }[status] || 'available';
}

// ── ASCII progress bar ───────────────────────────────────────────────────
function renderProgressBar() {
  if (!course) return;
  const total = course.chapters.length;
  const done = course.chapters.filter(c => (progress[c.id] || {}).status === 'complete').length;
  const filled = Math.round((done / total) * 20);
  const empty = 20 - filled;
  const bar = '[' + '█'.repeat(filled) + '░'.repeat(empty) + ']';
  progressBarAscii.textContent = `${bar} ${done}/${total}`;
}

// ── Render chapter list ──────────────────────────────────────────────────
function renderChapterList() {
  if (!course) return;
  chapterList.innerHTML = '';
  course.chapters.forEach((ch, i) => {
    const status = getChapterStatus(ch);
    const isInfo = ch.type === 'info';
    const li = document.createElement('li');
    li.className = 'chapter-item' +
      (status === 'locked' ? ' locked' : '') +
      (ch.id === activeChapterId ? ' active' : '') +
      (isInfo ? ' info' : '');

    li.innerHTML = `
      <span class="chapter-status ${isInfo ? 'info' : statusClass(status)}">${statusIcon(status, isInfo)}</span>
      <span class="chapter-text">
        <span class="chapter-num">${String(i + 1).padStart(2, '0')}</span>
        <span class="chapter-title">${ch.title}</span>
      </span>
    `;

    if (status !== 'locked') {
      li.addEventListener('click', () => loadChapter(ch.id));
    }

    chapterList.appendChild(li);
  });
  renderProgressBar();
}

// ── Load chapter content ─────────────────────────────────────────────────
async function loadChapter(chapterId) {
  if (!course) return;
  activeChapterId = chapterId;

  const chapter = course.chapters.find(c => c.id === chapterId);
  if (!chapter) return;

  const isInfo = chapter.type === 'info';

  // Ensure we're in course view
  showScreen('course');

  // Update chapter breadcrumb
  crumbChapterEl.textContent = chapter.title.toUpperCase();

  // Show loading state
  contentEmpty.style.display = 'none';
  contentBody.style.display = 'block';
  contentBody.innerHTML = '<p style="color:var(--dim)">// loading...</p>';

  // Always hide the next chapter button when entering a chapter; it will be
  // re-shown below (for info chapters) or by renderTestResults (for lessons).
  nextChapterBtn.classList.remove('visible');
  infoNextBtn.classList.remove('visible');

  // Info chapters: hide test UI entirely. Lesson chapters: restore previous results or hide.
  if (isInfo) {
    testResults.classList.remove('visible');
    runTestsBtn.style.display = 'none';
  } else {
    runTestsBtn.style.display = '';
    runTestsBtn.disabled = !currentProjectFolder;
    const saved = progress[chapterId];
    if (saved && saved.last_test_run) {
      renderTestResults(saved.last_test_run.results || [], saved.last_test_run);
    } else {
      testResults.classList.remove('visible');
    }
  }

  try {
    const md = await invoke('get_chapter_content', { chapterId });
    contentBody.innerHTML = marked.parse(md);
    contentBody.querySelectorAll('pre code').forEach(el => hljs.highlightElement(el));
  } catch (e) {
    contentBody.innerHTML = `<p style="color:var(--red)">// error loading chapter: ${e}</p>`;
  }

  // Info chapters auto-complete on view
  if (isInfo) {
    if (!progress[chapterId] || progress[chapterId].status !== 'complete') {
      progress[chapterId] = { status: 'complete', completed_at: new Date().toISOString() };
      await saveProgress();
    }
    // Show dedicated info next button if there is a next chapter
    const idx = course.chapters.findIndex(c => c.id === chapterId);
    if (idx >= 0 && idx < course.chapters.length - 1) {
      infoNextBtn.classList.add('visible');
    }
  }

  renderChapterList();
}

// ── Test runner ──────────────────────────────────────────────────────────

runTestsBtn.addEventListener('click', async () => {
  if (testRunning || !course || !currentProjectFolder || !activeChapterId) return;

  testRunning = true;
  runTestsBtn.disabled = true;

  // Show results panel in running state
  testResults.classList.add('visible');
  testRunningIndicator.classList.add('visible');
  testRows.innerHTML = '';
  testSummary.textContent = '';
  testSummary.className = '';
  nextChapterBtn.classList.remove('visible');
  serverCrashBanner.classList.remove('visible');
  serverCrashBanner.textContent = '';

  // Scroll test results into view
  testResults.scrollIntoView({ behavior: 'smooth', block: 'nearest' });

  try {
    // Get the test file data for the active chapter
    const tests = await invoke('get_chapter_tests', { chapterId: activeChapterId });

    // Find a free port dynamically (starts from default_port + 1 to avoid
    // colliding with any server the user may be running on the default port)
    const port = await invoke('find_free_port', {
      startPort: course.runtime.default_port + 1,
    });

    const result = await invoke('run_tests', {
      projectFolder: currentProjectFolder,
      serverCommand: course.runtime.server_command,
      port,
      pythonBin: await resolveVenvPython(),
      healthEndpoint: course.runtime.health_endpoint,
      cleanBeforeRun: course.runtime.clean_before_run,
      tests,
    });

    testRunningIndicator.classList.remove('visible');
    renderTestResults(result.results, result);

    // Persist progress
    const chapterProgress = progress[activeChapterId] || {};
    const allPassed = result.passed === result.total && result.total > 0;
    chapterProgress.last_test_run = {
      timestamp: new Date().toISOString(),
      passed: result.passed,
      failed: result.failed,
      total: result.total,
      results: result.results,
    };
    if (allPassed) {
      chapterProgress.status = 'complete';
      chapterProgress.completed_at = new Date().toISOString();
    } else if (result.total > 0) {
      chapterProgress.status = 'in_progress';
    }
    progress[activeChapterId] = chapterProgress;
    await saveProgress();
    renderChapterList();

  } catch (e) {
    testRunningIndicator.classList.remove('visible');
    testRows.innerHTML = `<div class="test-row">
      <span class="test-icon error">!</span>
      <span class="test-name" style="color:var(--orange)">${e}</span>
    </div>`;
    testSummary.textContent = 'test run failed';
    testSummary.className = 'has-fail';
  }

  testRunning = false;
  runTestsBtn.disabled = false;
});

// Resolve the venv python binary for the current project folder, or null.
async function resolveVenvPython() {
  if (!currentProjectFolder || !course) return null;
  try {
    const envStatus = await invoke('check_environment', {
      projectFolder: currentProjectFolder,
      versionMin: course.runtime.version_min,
      deps: [],
    });
    return envStatus.venv.found ? envStatus.venv.python_bin : null;
  } catch (_) {
    return null;
  }
}

function renderTestResults(results, summary) {
  testRows.innerHTML = '';
  serverCrashBanner.classList.remove('visible');
  nextChapterBtn.classList.remove('visible');

  if (!results || results.length === 0) {
    testRows.innerHTML = '<div class="test-row"><span class="test-icon" style="color:var(--dim)">—</span><span class="test-name">no results</span></div>';
    return;
  }

  results.forEach(r => {
    const iconCls = r.passed ? 'pass' : (r.error ? 'error' : 'fail');
    const iconCh = r.passed ? '✓' : (r.error ? '!' : '✗');

    let detailHtml = '';
    if (!r.passed && r.failures && r.failures.length > 0) {
      detailHtml = r.failures.map(f =>
        `<div class="test-detail fail">${escapeHtml(f)}</div>`
      ).join('');
    } else if (!r.passed && r.message) {
      detailHtml = `<div class="test-detail fail">${escapeHtml(r.message)}</div>`;
    }

    const row = document.createElement('div');
    row.className = 'test-row';
    row.innerHTML = `
      <span class="test-icon ${iconCls}">${iconCh}</span>
      <div style="flex:1">
        <div class="test-name">${escapeHtml(r.name)}</div>
        ${detailHtml}
      </div>
    `;
    testRows.appendChild(row);
  });

  if (summary) {
    const passed = summary.passed || 0;
    const total = summary.total || results.length;
    const failed = summary.failed || (total - passed);
    const allPassed = passed === total && total > 0;

    testSummary.textContent = `${passed}/${total} passed`;
    testSummary.className = allPassed ? 'all-pass' : 'has-fail';

    if (summary.server_crashed) {
      serverCrashBanner.textContent = 'Server crashed during test run. Check your code for errors.';
      serverCrashBanner.classList.add('visible');
    }

    if (allPassed) {
      // Check if there is a next chapter to advance to
      const idx = course.chapters.findIndex(c => c.id === activeChapterId);
      if (idx >= 0 && idx < course.chapters.length - 1) {
        nextChapterBtn.classList.add('visible');
      }
    }
  }
}

nextChapterBtn.addEventListener('click', () => {
  if (!course || !activeChapterId) return;
  const idx = course.chapters.findIndex(c => c.id === activeChapterId);
  if (idx >= 0 && idx < course.chapters.length - 1) {
    loadChapter(course.chapters[idx + 1].id);
  }
});

infoNextBtn.addEventListener('click', () => {
  if (!course || !activeChapterId) return;
  const idx = course.chapters.findIndex(c => c.id === activeChapterId);
  if (idx >= 0 && idx < course.chapters.length - 1) {
    loadChapter(course.chapters[idx + 1].id);
  }
});

function escapeHtml(str) {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

// ── Environment check (inline on overview) ───────────────────────────────

function iconFor(ok) {
  return ok ? { cls: 'ok', ch: '✓' } : { cls: 'fail', ch: '✗' };
}

// Run env check in background and render results inline on the overview.
// Does NOT block navigation — Enter Course goes straight to course view.
async function refreshEnvStatus(folder) {
  if (!course || !folder) return;

  overviewEnvStatus.style.display = '';
  overviewEnvOutput.classList.remove('visible');
  overviewEnvOutput.textContent = '';
  overviewEnvRows.innerHTML = `
    <div class="env-row">
      <span class="env-icon loading">...</span>
      <span class="env-label">Checking environment</span>
    </div>`;
  overviewEnvActions.innerHTML = '';

  let status;
  try {
    status = await invoke('check_environment', {
      projectFolder: folder,
      versionMin: course.runtime.version_min,
      deps: [],
    });
  } catch (e) {
    overviewEnvRows.innerHTML = `<div class="env-row">
      <span class="env-icon fail">✗</span>
      <span class="env-label">Environment check failed<div class="env-detail">${escapeHtml(String(e))}</div></span>
    </div>`;
    return;
  }

  renderOverviewEnvStatus(status, folder);
}

function renderOverviewEnvStatus(status, folder) {
  const { python, venv } = status;

  // If everything looks fine, hide the env section entirely
  const pyOk = python.found && python.meets_min;
  if (pyOk && venv.found) {
    overviewEnvStatus.style.display = 'none';
    return;
  }

  let html = '';

  // Python row
  const pyIcon = iconFor(pyOk);
  let pyDetail = '';
  if (!python.found) {
    pyDetail = `Python ${course.runtime.version_min}+ not found — install from <a href="#" class="env-ext-link" data-url="https://python.org/downloads">python.org</a> or use <a href="#" class="env-ext-link" data-url="https://github.com/pyenv/pyenv">pyenv</a>`;
  } else if (!python.meets_min) {
    pyDetail = `Found ${python.version} — need ${course.runtime.version_min}+`;
  } else {
    pyDetail = python.version;
  }
  html += `<div class="env-row">
    <span class="env-icon ${pyIcon.cls}">${pyIcon.ch}</span>
    <span class="env-label">Python<div class="env-detail">${pyDetail}</div></span>
  </div>`;

  // Venv row
  const venvIcon = iconFor(venv.found);
  const venvDetail = venv.found ? venv.path : 'No venv found — create one or set up pyenv in this folder';
  let venvAction = '';
  if (!venv.found && python.found && python.meets_min) {
    venvAction = `<button class="env-action overview-env-action" id="ov-create-venv-btn" data-pybin="${escapeHtml(python.binary)}">[ Create venv ]</button>`;
  }
  html += `<div class="env-row">
    <span class="env-icon ${venvIcon.cls}">${venvIcon.ch}</span>
    <span class="env-label">Virtual environment<div class="env-detail">${venvDetail}</div></span>
    ${venvAction}
  </div>`;

  overviewEnvRows.innerHTML = html;

  // Wire external links
  overviewEnvRows.querySelectorAll('.env-ext-link').forEach(a => {
    a.addEventListener('click', e => {
      e.preventDefault();
      invoke('open_url', { url: a.dataset.url }).catch(() => { });
    });
  });

  // Re-check button
  overviewEnvActions.innerHTML = `<button class="env-action overview-env-action" id="ov-recheck-btn">[ Re-check ]</button>`;

  const recheckBtn = document.getElementById('ov-recheck-btn');
  recheckBtn.addEventListener('click', () => refreshEnvStatus(folder));

  // Create venv button
  const createVenvBtn = document.getElementById('ov-create-venv-btn');
  if (createVenvBtn) {
    createVenvBtn.addEventListener('click', async () => {
      createVenvBtn.disabled = true;
      createVenvBtn.textContent = '[ Creating... ]';
      try {
        const result = await invoke('create_venv', {
          projectFolder: folder,
          pythonBin: createVenvBtn.dataset.pybin,
        });
        overviewEnvOutput.textContent = `venv created at ${result}`;
        overviewEnvOutput.classList.add('visible');
      } catch (e) {
        overviewEnvOutput.textContent = `Error: ${e}`;
        overviewEnvOutput.classList.add('visible');
      }
      await refreshEnvStatus(folder);
    });
  }
}

// ── Course overview rendering ─────────────────────────────────────────────

function renderOverview() {
  if (!course) return;
  overviewLoadError.style.display = 'none';
  overviewTitle.textContent = course.title;
  overviewDesc.textContent = course.description || '';

  // Tech stack chips from runtime
  const techItems = [
    course.runtime.name,
    `Python ${course.runtime.version_min}+`,
    ...(course.runtime.dependencies || []).slice(0, 4),
  ].filter(Boolean);
  overviewTech.innerHTML = techItems
    .map(t => `<span class="tech-chip">${escapeHtml(t)}</span>`)
    .join('');

  // Chapter list preview
  overviewChList.innerHTML = course.chapters.map((ch, i) => {
    const done = (progress[ch.id] || {}).status === 'complete';
    return `<li class="overview-ch-item">
      <span class="overview-ch-num">${String(i + 1).padStart(2, '0')}</span>
      <span class="overview-ch-title">${escapeHtml(ch.title)}</span>
      ${done ? '<span class="overview-ch-done">✓</span>' : ''}
    </li>`;
  }).join('');

  // Folder path display + button states
  const enterBtn = document.getElementById('overview-enter-btn');
  if (currentProjectFolder) {
    const parts = currentProjectFolder.replace(/\/$/, '').split('/');
    const name = parts.pop();
    overviewFolderPath.innerHTML =
      `folder: <span class="folder-name">${escapeHtml(name)}</span>` +
      ` <span style="color:#333">${escapeHtml(parts.join('/') + '/')}</span>`;
    overviewFolderPath.style.display = '';
    overviewSelFolderBtn.textContent = '[ Change Folder ]';
    overviewSelFolderBtn.classList.add('secondary');
    enterBtn.style.display = 'inline-block';
  } else {
    overviewFolderPath.style.display = 'none';
    overviewSelFolderBtn.textContent = '[ Select Project Folder ]';
    overviewSelFolderBtn.classList.remove('secondary');
    enterBtn.style.display = 'none';
  }
}

// ── Folder selection ─────────────────────────────────────────────────────

async function selectProjectFolder() {
  try {
    const folder = await invoke('select_folder');
    if (!folder) return;
    currentProjectFolder = folder;
    await invoke('save_folder', { courseId: course.id, folder });
    await loadProgress();
    renderChapterList();
    renderOverview();
    // Run env check in background to show inline status
    refreshEnvStatus(folder);
  } catch (e) {
    console.error('select_folder failed:', e);
  }
}

overviewSelFolderBtn.addEventListener('click', selectProjectFolder);

document.getElementById('overview-enter-btn').addEventListener('click', () => {
  if (currentProjectFolder) {
    showScreen('course');
    if (!activeChapterId) {
      contentEmpty.style.display = '';
      contentBody.style.display = 'none';
      testResults.classList.remove('visible');
    }
  }
});

document.getElementById('overview-back-to-menu-btn').addEventListener('click', () => showMenu());


openTerminalBtn.addEventListener('click', async () => {
  if (!currentProjectFolder) return;
  try { await invoke('open_terminal', { folder: currentProjectFolder }); }
  catch (e) { console.error('open_terminal failed:', e); }
});

// ── Load course helpers ───────────────────────────────────────────────────

async function onCourseLoaded(loadedCourse) {
  course = loadedCourse;
  updatePostmanPort();

  await loadProgress();
  renderChapterList();

  // Check for saved folder for this course
  const saved = await invoke('get_saved_folder', { courseId: course.id });
  if (saved) {
    currentProjectFolder = saved;
  }
  renderOverview();
  showScreen('overview');

  // If a folder is already set, run env check in background to show inline status
  if (currentProjectFolder) {
    refreshEnvStatus(currentProjectFolder);
  }
}

async function loadDifferentCourse() {
  try {
    const loaded = await invoke('pick_and_load_course');
    if (loaded) {
      currentProjectFolder = null;
      activeChapterId = null;
      progress = {};
      menuAllExpanded = false;
      await onCourseLoaded(loaded);
    }
  } catch (e) {
    showCourseLoadError(String(e));
  }
}

function showCourseLoadError(message) {
  // Show on overview if we're there, otherwise show on menu as an alert-style banner
  if (course) {
    renderOverview();
    showScreen('overview');
    overviewLoadError.textContent = `Failed to load course: ${message}`;
    overviewLoadError.style.display = '';
  } else {
    // Show a temporary error on the menu screen
    const existing = document.getElementById('menu-load-error');
    if (existing) existing.remove();
    const err = document.createElement('div');
    err.id = 'menu-load-error';
    err.className = 'menu-load-error';
    err.textContent = `Failed to load course: ${message}`;
    document.getElementById('menu-body').prepend(err);
    setTimeout(() => err.remove(), 6000);
  }
}

// ── Menu screen ───────────────────────────────────────────────────────────

const MAX_QUICK_SLOTS = 3;
let allInstalled = [];
let menuAllExpanded = false;

async function showMenu() {
  try {
    allInstalled = await invoke('get_installed_courses') || [];
  } catch (_) {
    allInstalled = [];
  }
  renderMenuCards();
  showScreen('menu');
}

function renderMenuCards() {
  const recent = allInstalled.slice(0, MAX_QUICK_SLOTS);

  // Render quick-access cards
  menuCards.innerHTML = '';
  recent.forEach(c => {
    const el = document.createElement('div');
    el.className = 'menu-course-card';
    const total = c.chapters_total;
    const done = c.chapters_complete;
    const pct = total > 0 ? done / total : 0;
    const filled = Math.round(pct * 12);
    const bar = '[' + '█'.repeat(filled) + '░'.repeat(12 - filled) + ']';
    el.innerHTML = `
      <div class="card-id">${escapeHtml(c.id)}</div>
      <div class="card-title">${escapeHtml(c.title || c.id)}</div>
      <div class="card-progress-bar">${bar}</div>
      <div class="card-progress-text">${done}/${total} chapters</div>
    `;
    el.addEventListener('click', () => openInstalledCourse(c.id));
    menuCards.appendChild(el);
  });

  // Fill remaining quick slots with empty + add slot
  const emptySlots = MAX_QUICK_SLOTS - recent.length;
  for (let i = 0; i < emptySlots; i++) {
    const el = document.createElement('div');
    el.className = 'menu-card-empty';
    el.textContent = '[ + Load Course ]';
    el.addEventListener('click', loadDifferentCourse);
    menuCards.appendChild(el);
  }

  // Show/hide "Show All" button
  if (allInstalled.length > MAX_QUICK_SLOTS) {
    menuShowAllBtn.style.display = '';
    menuShowAllBtn.textContent = menuAllExpanded
      ? '[ Hide ]'
      : `[ Show All Courses (${allInstalled.length}) ]`;
    if (menuAllExpanded) renderMenuAll();
  } else {
    menuShowAllBtn.style.display = 'none';
    menuAllCourses.style.display = 'none';
  }
}

function renderMenuAll() {
  menuAllCourses.style.display = '';
  menuAllCourses.innerHTML = allInstalled.map((c, i) => {
    const done = c.chapters_complete;
    const total = c.chapters_total;
    return `<div class="menu-all-row" data-course-idx="${i}">
      <div class="menu-all-title">${escapeHtml(c.title || c.id)}</div>
      <div class="menu-all-progress">${done}/${total}</div>
    </div>`;
  }).join('');

  menuAllCourses.querySelectorAll('.menu-all-row').forEach(row => {
    row.addEventListener('click', () => {
      const c = allInstalled[parseInt(row.dataset.courseIdx)];
      openInstalledCourse(c.id);
    });
  });
}

async function openInstalledCourse(courseId) {
  // Load from extracted path in ~/.praxis/courses/{id}/
  try {
    const coursePath = await invoke('get_installed_course_path', { courseId });
    const loaded = await invoke('load_course', { path: coursePath });
    currentProjectFolder = null;
    activeChapterId = null;
    progress = {};
    menuAllExpanded = false;
    await onCourseLoaded(loaded);
  } catch (e) {
    // Course files not found locally — prompt to reload from .course file
    const errMsg = String(e);
    const isNotFound = errMsg.toLowerCase().includes('not found') || errMsg.toLowerCase().includes('no such file');
    if (isNotFound) {
      // Silently fall through to file picker — the extracted files are just gone
      await loadDifferentCourse();
    } else {
      showCourseLoadError(errMsg);
    }
  }
}

menuShowAllBtn.addEventListener('click', () => {
  menuAllExpanded = !menuAllExpanded;
  if (menuAllExpanded) {
    menuShowAllBtn.textContent = '[ Hide ]';
    menuAllCourses.style.display = '';
    renderMenuAll();
  } else {
    menuShowAllBtn.textContent = `[ Show All Courses (${allInstalled.length}) ]`;
    menuAllCourses.style.display = 'none';
  }
});

menuLoadCourseBtn.addEventListener('click', loadDifferentCourse);

// ── Breadcrumb navigation ─────────────────────────────────────────────────

crumbPraxis.addEventListener('click', () => showMenu());

crumbCourse.addEventListener('click', () => {
  if (!course) return;
  renderOverview();
  showScreen('overview');
});

// ── Init ─────────────────────────────────────────────────────────────────

async function init() {
  await showMenu();
}

loadCourseBtn.addEventListener('click', loadDifferentCourse);

init();

// ── Postman Lite ─────────────────────────────────────────────────────────

const app = document.getElementById('app');
const postmanToggle = document.getElementById('postman-toggle');
const postmanToggleArr = document.getElementById('postman-toggle-arrow');
const postmanBodyEl = document.getElementById('postman-body');
const postmanMethod = document.getElementById('postman-method');
const postmanUrl = document.getElementById('postman-url');
const postmanSendBtn = document.getElementById('postman-send-btn');
const postmanSaveBtn = document.getElementById('postman-save-btn');
const postmanSending = document.getElementById('postman-sending');
const postmanPortBadge = document.getElementById('postman-port-badge');
const headerRows = document.getElementById('postman-header-rows');
const addHeaderBtn = document.getElementById('postman-add-header-btn');
const addAuthBtn = document.getElementById('postman-add-auth-btn');
const bodyTextarea = document.getElementById('postman-body-textarea');
const responseEmpty = document.getElementById('postman-response-empty');
const responseContent = document.getElementById('postman-response-content');
const statusBadge = document.getElementById('postman-status-badge');
const statusText = document.getElementById('postman-status-text');
const respHeadersToggle = document.getElementById('postman-resp-headers-toggle');
const respHeadersEl = document.getElementById('postman-resp-headers');
const respBodyEl = document.getElementById('postman-resp-body');
const histHistoryPane = document.getElementById('hist-history');
const histSavedPane = document.getElementById('hist-saved');

let postmanOpen = false;
let requestHistory = [];   // [{ method, url, headers, body }]
let savedRequests = [];    // loaded from progress.json
let respHeadersVisible = false;
let postmanSending_ = false;

// ── Toggle open/close ────────────────────────────────────────────────────
postmanToggle.addEventListener('click', () => {
  postmanOpen = !postmanOpen;
  app.classList.toggle('postman-open', postmanOpen);
  postmanToggle.classList.toggle('open', postmanOpen);
  postmanToggleArr.textContent = postmanOpen ? '▼' : '▲';
  if (postmanOpen) postmanBodyEl.style.display = 'flex';
  else postmanBodyEl.style.display = 'none';
});

// Hide body initially
postmanBodyEl.style.display = 'none';

// ── Update port badge ────────────────────────────────────────────────────
function updatePostmanPort() {
  if (course) {
    const port = course.runtime.default_port;
    postmanPortBadge.textContent = `:${port}`;
    // Pre-fill URL if it's blank or just placeholder
    if (!postmanUrl.value || postmanUrl.value === postmanUrl.placeholder) {
      postmanUrl.value = `http://localhost:${port}/`;
      postmanUrl.placeholder = `http://localhost:${port}/`;
    }
  }
}

// ── Tab switching ────────────────────────────────────────────────────────
document.querySelectorAll('.ptab').forEach(tab => {
  tab.addEventListener('click', () => {
    document.querySelectorAll('.ptab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.ptab-pane').forEach(p => p.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById('ptab-' + tab.dataset.tab).classList.add('active');
  });
});

document.querySelectorAll('.hist-tab').forEach(tab => {
  tab.addEventListener('click', () => {
    document.querySelectorAll('.hist-tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.hist-pane').forEach(p => p.classList.remove('active'));
    tab.classList.add('active');
    document.getElementById('hist-' + tab.dataset.hist).classList.add('active');
  });
});

// ── Headers management ───────────────────────────────────────────────────
function addHeaderRow(key = '', val = '') {
  const row = document.createElement('div');
  row.className = 'header-row';
  row.innerHTML = `
    <input class="header-key" type="text" placeholder="Key" value="${escapeHtml(key)}" spellcheck="false">
    <input class="header-val" type="text" placeholder="Value" value="${escapeHtml(val)}" spellcheck="false">
    <button class="header-del-btn" title="Remove">x</button>
  `;
  row.querySelector('.header-del-btn').addEventListener('click', () => row.remove());
  headerRows.appendChild(row);
}

function getHeaders() {
  const headers = {};
  headerRows.querySelectorAll('.header-row').forEach(row => {
    const k = row.querySelector('.header-key').value.trim();
    const v = row.querySelector('.header-val').value.trim();
    if (k) headers[k] = v;
  });
  return headers;
}

function setHeaders(headers) {
  headerRows.innerHTML = '';
  Object.entries(headers).forEach(([k, v]) => addHeaderRow(k, v));
}

// Default headers
addHeaderRow('Content-Type', 'application/json');

addHeaderBtn.addEventListener('click', () => addHeaderRow());

addAuthBtn.addEventListener('click', () => {
  // Check if Authorization row already exists
  const existing = Array.from(headerRows.querySelectorAll('.header-key'))
    .find(el => el.value.trim().toLowerCase() === 'authorization');
  if (existing) {
    existing.closest('.header-row').querySelector('.header-val').value = 'Bearer ';
    existing.closest('.header-row').querySelector('.header-val').focus();
  } else {
    addHeaderRow('Authorization', 'Bearer ');
    // Focus the value field of the new row
    const rows = headerRows.querySelectorAll('.header-row');
    rows[rows.length - 1].querySelector('.header-val').focus();
  }
  // Select Bearer text so user can type the token directly
  setTimeout(() => {
    const vals = headerRows.querySelectorAll('.header-val');
    const last = Array.from(vals).find(el => el.value.startsWith('Bearer '));
    if (last) {
      last.setSelectionRange(7, 7);
    }
  }, 20);
});

// ── Response headers toggle ──────────────────────────────────────────────
respHeadersToggle.addEventListener('click', () => {
  respHeadersVisible = !respHeadersVisible;
  respHeadersEl.classList.toggle('visible', respHeadersVisible);
  respHeadersToggle.textContent = respHeadersVisible ? 'headers ▴' : 'headers ▾';
});

// ── Send request ─────────────────────────────────────────────────────────
postmanSendBtn.addEventListener('click', sendRequest);

postmanUrl.addEventListener('keydown', e => {
  if (e.key === 'Enter') sendRequest();
});

async function sendRequest() {
  if (postmanSending_) return;
  const method = postmanMethod.value;
  const url = postmanUrl.value.trim();
  if (!url) return;

  const headers = getHeaders();
  const body = bodyTextarea.value.trim() || null;

  postmanSending_ = true;
  postmanSendBtn.disabled = true;
  postmanSending.classList.add('visible');

  // Switch to response tab
  document.querySelectorAll('.ptab').forEach(t => t.classList.remove('active'));
  document.querySelectorAll('.ptab-pane').forEach(p => p.classList.remove('active'));
  document.querySelector('.ptab[data-tab="response"]').classList.add('active');
  document.getElementById('ptab-response').classList.add('active');

  try {
    const resp = await invoke('send_http_request', { method, url, headers, body });

    showResponse(resp);

    // Add to history
    const entry = { method, url, headers, body, timestamp: new Date().toISOString() };
    requestHistory.unshift(entry);
    if (requestHistory.length > 50) requestHistory.pop();
    renderHistory();

  } catch (e) {
    responseEmpty.style.display = 'none';
    responseContent.classList.add('visible');
    statusBadge.textContent = 'ERR';
    statusBadge.className = 'sother';
    statusText.textContent = String(e);
    respBodyEl.textContent = '';
    respHeadersEl.innerHTML = '';
  }

  postmanSending_ = false;
  postmanSendBtn.disabled = false;
  postmanSending.classList.remove('visible');
}

function showResponse(resp) {
  responseEmpty.style.display = 'none';
  responseContent.classList.add('visible');

  // Status badge
  const s = resp.status;
  statusBadge.textContent = s;
  if (s >= 200 && s < 300) statusBadge.className = 'postman-status-badge s2xx';
  else if (s >= 400 && s < 500) statusBadge.className = 'postman-status-badge s4xx';
  else if (s >= 500) statusBadge.className = 'postman-status-badge s5xx';
  else statusBadge.className = 'postman-status-badge sother';

  statusText.textContent = httpStatusText(s);

  // Response headers
  respHeadersEl.innerHTML = Object.entries(resp.headers).map(([k, v]) =>
    `<div class="resp-header-row">
      <span class="resp-header-key">${escapeHtml(k)}</span>
      <span class="resp-header-val">${escapeHtml(v)}</span>
    </div>`
  ).join('');

  // Response body — try to pretty-print JSON
  let bodyText = resp.body;
  try {
    const parsed = JSON.parse(resp.body);
    bodyText = JSON.stringify(parsed, null, 2);
  } catch (_) { /* not JSON, show raw */ }
  respBodyEl.textContent = bodyText;
}

function httpStatusText(code) {
  const map = {
    200: 'OK', 201: 'Created', 204: 'No Content', 400: 'Bad Request',
    401: 'Unauthorized', 403: 'Forbidden', 404: 'Not Found',
    405: 'Method Not Allowed', 409: 'Conflict', 422: 'Unprocessable Entity',
    500: 'Internal Server Error', 502: 'Bad Gateway', 503: 'Service Unavailable',
  };
  return map[code] || '';
}

// ── History rendering ────────────────────────────────────────────────────
function renderHistory() {
  if (requestHistory.length === 0) {
    histHistoryPane.innerHTML = '<div class="hist-empty">// no requests yet</div>';
    return;
  }
  histHistoryPane.innerHTML = requestHistory.map((entry, i) => {
    const path = (() => { try { return new URL(entry.url).pathname; } catch { return entry.url; } })();
    return `<div class="hist-item" data-idx="${i}">
      <span class="hist-method ${entry.method}">${entry.method}</span>
      <span class="hist-path" title="${escapeHtml(entry.url)}">${escapeHtml(path)}</span>
    </div>`;
  }).join('');

  histHistoryPane.querySelectorAll('.hist-item').forEach(item => {
    item.addEventListener('click', () => {
      const entry = requestHistory[parseInt(item.dataset.idx)];
      loadRequestIntoForm(entry);
    });
  });
}

// ── Saved requests ───────────────────────────────────────────────────────
async function loadSavedRequests() {
  try {
    savedRequests = await invoke('get_saved_requests') || [];
  } catch (_) {
    savedRequests = [];
  }
  renderSaved();
}

function renderSaved() {
  if (savedRequests.length === 0) {
    histSavedPane.innerHTML = '<div class="hist-empty">// no saved requests</div>';
    return;
  }
  histSavedPane.innerHTML = savedRequests.map((r, i) =>
    `<div class="hist-item" data-saved-idx="${i}">
      <span class="hist-method ${r.method}">${r.method}</span>
      <span class="hist-path" title="${escapeHtml(r.url)}">${escapeHtml(r.name || r.url)}</span>
      <button class="hist-del" data-saved-id="${escapeHtml(r.id)}" title="Delete">x</button>
    </div>`
  ).join('');

  histSavedPane.querySelectorAll('.hist-item').forEach(item => {
    item.addEventListener('click', e => {
      if (e.target.classList.contains('hist-del')) return;
      const r = savedRequests[parseInt(item.dataset.savedIdx)];
      loadRequestIntoForm(r);
    });
  });

  histSavedPane.querySelectorAll('.hist-del').forEach(btn => {
    btn.addEventListener('click', async e => {
      e.stopPropagation();
      const id = btn.dataset.savedId;
      try {
        await invoke('delete_saved_request', { id });
        savedRequests = savedRequests.filter(r => r.id !== id);
        renderSaved();
      } catch (_) { }
    });
  });
}

function loadRequestIntoForm(r) {
  postmanMethod.value = r.method || 'GET';
  postmanUrl.value = r.url || '';
  if (r.headers) setHeaders(r.headers);
  bodyTextarea.value = r.body || '';
}

postmanSaveBtn.addEventListener('click', async () => {
  const method = postmanMethod.value;
  const url = postmanUrl.value.trim();
  if (!url) return;

  const defaultName = `${method} ${(() => { try { return new URL(url).pathname; } catch { return url; } })()}`;
  const name = window.prompt('Save request as:', defaultName);
  if (!name) return;

  const id = 'req_' + Date.now();
  const req = {
    id, name, method, url,
    headers: getHeaders(),
    body: bodyTextarea.value.trim() || null,
  };

  try {
    await invoke('save_request', { request: req });
    savedRequests.unshift(req);
    renderSaved();
    // Switch to saved tab
    document.querySelectorAll('.hist-tab').forEach(t => t.classList.remove('active'));
    document.querySelectorAll('.hist-pane').forEach(p => p.classList.remove('active'));
    document.querySelector('.hist-tab[data-hist="saved"]').classList.add('active');
    document.getElementById('hist-saved').classList.add('active');
  } catch (_) { }
});

// Load saved requests on startup (after init)
loadSavedRequests();

// ── Slide-over panels ─────────────────────────────────────────────────────

const backdrop = document.getElementById('slide-over-backdrop');
const courseSettingsPanel = document.getElementById('course-settings-panel');
const appSettingsPanel = document.getElementById('app-settings-panel');
const overviewSettingsBtn = document.getElementById('overview-settings-btn');

function openSlideOver(panel) {
  closeAllSlideOvers();
  panel.classList.add('open');
  backdrop.classList.add('visible');
}

function closeAllSlideOvers() {
  courseSettingsPanel.classList.remove('open');
  appSettingsPanel.classList.remove('open');
  backdrop.classList.remove('visible');
  // Hide confirm dialog
  document.getElementById('cs-reset-confirm').style.display = 'none';
}

backdrop.addEventListener('click', closeAllSlideOvers);

document.getElementById('course-settings-close').addEventListener('click', closeAllSlideOvers);
document.getElementById('app-settings-close').addEventListener('click', closeAllSlideOvers);

function openCourseSettings() {
  if (!course) return;
  const total = course.chapters.length;
  const done = course.chapters.filter(c => (progress[c.id] || {}).status === 'complete').length;
  document.getElementById('cs-course-title').textContent = course.title;
  document.getElementById('cs-course-version').textContent = `v${course.version}`;
  document.getElementById('cs-progress-detail').textContent = `${done} of ${total} chapters complete`;
  document.getElementById('cs-reset-confirm').style.display = 'none';
  openSlideOver(courseSettingsPanel);
}

overviewSettingsBtn.addEventListener('click', openCourseSettings);

document.getElementById('cs-reset-btn').addEventListener('click', () => {
  document.getElementById('cs-reset-confirm').style.display = '';
});

document.getElementById('cs-reset-confirm-no').addEventListener('click', () => {
  document.getElementById('cs-reset-confirm').style.display = 'none';
});

document.getElementById('cs-reset-confirm-yes').addEventListener('click', async () => {
  if (!course) return;
  try {
    await invoke('reset_course_progress', { courseId: course.id });
    progress = {};
    activeChapterId = null;
    renderChapterList();
    renderOverview();
    document.getElementById('cs-progress-detail').textContent = '0 of ' + course.chapters.length + ' chapters complete';
    document.getElementById('cs-reset-confirm').style.display = 'none';
  } catch (e) {
    console.error('reset_course_progress failed:', e);
  }
});

async function openAppSettings() {
  let installed = [];
  try {
    installed = await invoke('get_installed_courses') || [];
  } catch (_) { }

  const list = document.getElementById('as-course-list');
  if (installed.length === 0) {
    list.innerHTML = '<div class="slide-over-sub">No courses installed.</div>';
  } else {
    list.innerHTML = installed.map((c, i) =>
      `<div class="as-course-row" data-idx="${i}">
        <div class="as-course-info">
          <div class="as-course-name">${escapeHtml(c.title || c.id)}</div>
          <div class="as-course-meta">${c.chapters_complete}/${c.chapters_total} chapters &middot; v${escapeHtml(c.version)}</div>
        </div>
        <button class="as-course-remove-btn" data-course-id="${escapeHtml(c.id)}">[ Remove ]</button>
      </div>`
    ).join('');

    list.querySelectorAll('.as-course-remove-btn').forEach(btn => {
      btn.addEventListener('click', () => {
        const id = btn.dataset.courseId;
        const row = btn.closest('.as-course-row');

        // Replace row content with inline confirmation
        row.innerHTML = `
          <div class="as-course-remove-confirm">
            <span class="as-course-remove-confirm-text">Remove this course and all progress?</span>
            <div class="as-course-remove-confirm-actions">
              <button class="slide-over-danger-btn as-confirm-yes" data-course-id="${escapeHtml(id)}">[ Remove ]</button>
              <button class="slide-over-cancel-btn as-confirm-no">[ Cancel ]</button>
            </div>
          </div>
        `;

        row.querySelector('.as-confirm-no').addEventListener('click', () => openAppSettings());

        row.querySelector('.as-confirm-yes').addEventListener('click', async () => {
          try {
            await invoke('remove_course', { courseId: id });
            if (course && course.id === id) {
              course = null;
              progress = {};
              activeChapterId = null;
              currentProjectFolder = null;
            }
            closeAllSlideOvers();
            await showMenu();
          } catch (e) {
            console.error('remove_course failed:', e);
          }
        });
      });
    });
  }

  openSlideOver(appSettingsPanel);
}

menuAppSettingsBtn.addEventListener('click', openAppSettings);
