(() => {
  'use strict';

  const $ = (id) => document.getElementById(id);
  const icon = (name) => `<svg class="icon" aria-hidden="true"><use href="#icon-${name}"/></svg>`;
  const escape = (value) => String(value).replace(/[&<>"']/g, (char) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[char]));
  const elements = {
    app: $('app'), workspace: $('workspace'), panes: $('panes'), dividers: $('dividers'),
    tabs: $('tabs'), picker: $('picker'), templates: $('templates'), menu: $('splitMenu'),
    ghost: $('ghost'), empty: $('empty'),
  };
  let nextPaneId = 0, nextTabId = 0, nextSplitId = 0;
  const paneInfo = new Map();
  const paneElements = new Map();
  const tabElements = new Map();
  const dividerElements = new Map();

  // Identity belongs to the session and split, independently of visual position.
  const leaf = (pane) => ({ type: 'leaf', pane });
  const split = (axis, ratio, a, b) => ({ type: 'split', id: nextSplitId++, axis, ratio, a, b });
  function leaves(node) {
    return !node ? [] : node.type === 'leaf' ? [node.pane] : [...leaves(node.a), ...leaves(node.b)];
  }
  function replacePane(node, pane, transform) {
    if (!node) return null;
    if (node.type === 'leaf') return node.pane === pane ? transform(node) : node;
    const a = replacePane(node.a, pane, transform), b = replacePane(node.b, pane, transform);
    return !a ? b : !b ? a : { ...node, a, b };
  }
  function swapPanes(node, a, b) {
    if (!node) return null;
    if (node.type === 'leaf') return leaf(node.pane === a ? b : node.pane === b ? a : node.pane);
    return { ...node, a: swapPanes(node.a, a, b), b: swapPanes(node.b, a, b) };
  }
  function even(axis, ids) {
    return ids.length === 1 ? leaf(ids[0]) : split(axis, 1 / ids.length, leaf(ids[0]), even(axis, ids.slice(1)));
  }
  function geometry(node, box = [0, 0, 1, 1], result = { panes: new Map(), splits: new Map() }) {
    if (!node) return result;
    if (node.type === 'leaf') { result.panes.set(node.pane, box); return result; }
    const [x, y, w, h] = box, r = node.ratio;
    result.splits.set(node.id, { node, box });
    if (node.axis === 'row') {
      geometry(node.a, [x, y, w * r, h], result);
      geometry(node.b, [x + w * r, y, w * (1 - r), h], result);
    } else {
      geometry(node.a, [x, y, w, h * r], result);
      geometry(node.b, [x, y + h * r, w, h * (1 - r)], result);
    }
    return result;
  }
  function templatesFor(ids) {
    if (!ids.length) return [];
    if (ids.length === 1) return [{ name: '整窗', build: (p) => leaf(p[0]) }];
    if (ids.length === 2) return [
      { name: '左右', build: (p) => split('row', .5, leaf(p[0]), leaf(p[1])) },
      { name: '上下', build: (p) => split('col', .5, leaf(p[0]), leaf(p[1])) },
      { name: '主次', build: (p) => split('row', 2 / 3, leaf(p[0]), leaf(p[1])) },
    ];
    if (ids.length === 3) return [
      { name: '主次', build: (p) => split('row', 2 / 3, leaf(p[0]), split('col', .5, leaf(p[1]), leaf(p[2]))) },
      { name: '左二右一', build: (p) => split('row', .5, split('col', .5, leaf(p[0]), leaf(p[1])), leaf(p[2])) },
      { name: '三列', build: (p) => even('row', p) },
      { name: '三行', build: (p) => even('col', p) },
      { name: '上一下二', build: (p) => split('col', .5, leaf(p[0]), split('row', .5, leaf(p[1]), leaf(p[2]))) },
    ];
    if (ids.length === 4) return [
      { name: '四宫格', build: (p) => split('row', .5, split('col', .5, leaf(p[0]), leaf(p[1])), split('col', .5, leaf(p[2]), leaf(p[3]))) },
      { name: '主次', build: (p) => split('row', 2 / 3, leaf(p[0]), even('col', p.slice(1))) },
      { name: '四列', build: (p) => even('row', p) },
      { name: '四行', build: (p) => even('col', p) },
    ];
    return [
      { name: '网格', build: (p) => {
        const middle = Math.ceil(p.length / 2);
        return split('row', .5, even('col', p.slice(0, middle)), even('col', p.slice(middle)));
      } },
      { name: '横排', build: (p) => even('row', p) },
      { name: '竖排', build: (p) => even('col', p) },
    ];
  }

  const state = { tabs: [], activeTab: null, overlay: null, preview: null, drag: null, resize: null, swapTarget: null, hot: false, review: 'workspace' };
  const tab = () => state.tabs.find((item) => item.id === state.activeTab);
  function newPane(kind = 'shell', path = 'D:\\workspace\\Glim') {
    const id = nextPaneId++;
    paneInfo.set(id, { kind, path, shell: 'PowerShell' });
    return id;
  }
  function newTab(title = '终端', path) {
    const pane = newPane('shell', path);
    return { id: nextTabId++, title, tree: leaf(pane), active: pane, zoomed: null };
  }
  const initial = newTab('glim');
  const server = newPane('server'), tests = newPane('tests');
  initial.tree = split('row', 2 / 3, initial.tree, split('col', .5, leaf(server), leaf(tests)));
  const website = newTab('website', 'D:\\workspace\\website');
  state.tabs.push(initial, website);
  state.activeTab = initial.id;
  const baseline = JSON.stringify(state.tabs);

  function terminalHTML(info) {
    const line = (text, kind = '') => `<div class="terminal-line ${kind}">${text}</div>`;
    const command = (text) => line(`<span class="command-symbol">❯</span>${escape(text)}`, 'terminal-command');
    const gap = '<div class="terminal-gap"></div>';
    const path = info.path.split('\\').at(-1);
    const prompt = `<div class="terminal-prompt"><span>${escape(path)}</span><span class="prompt-branch">${icon('branch')}main</span></div><div class="terminal-line prompt-line"><span class="command-symbol">❯</span><span class="caret"></span></div>`;
    if (info.kind === 'server') return [
      command('pnpm dev'), gap,
      line('> glim-web@0.1.0 dev', 'terminal-comment'), line('> vite', 'terminal-comment'), gap,
      line('VITE  ready in 184 ms', 'terminal-success'), gap,
      line('  Local:   <span class="terminal-info">http://localhost:5173/</span>'),
      line('  Network: use --host to expose', 'terminal-comment'), gap,
      line('<span class="output-time">15:42:08</span>page reload index.html', 'terminal-comment'),
      line('<span class="output-time">15:42:11</span>hmr update /src/app.css', 'terminal-comment'),
    ].join('');
    if (info.kind === 'tests') return [
      command('cargo test'),
      line('running 4 tests', 'terminal-comment'), gap,
      line('test workspace::split ... <span class="terminal-success">ok</span>'),
      line('test workspace::swap ... <span class="terminal-success">ok</span>'),
      line('test layout::apply ... <span class="terminal-success">ok</span>'),
      line('test layout::restore ... <span class="terminal-success">ok</span>'), gap,
      line('test result: ok. 4 passed; 0 failed', 'terminal-success'), prompt,
    ].join('');
    return [
      command('git status --short'),
      line(' <span class="terminal-warning">M</span> src/workspace.rs'),
      line(' <span class="terminal-warning">M</span> src/theme.rs'),
      line('<span class="terminal-success">??</span> src/terminal/'), gap,
      command('cargo check'), gap,
      line('<span class="output-key">Checking</span>glim v0.1.0'),
      line(`<span class="terminal-comment">         (${escape(info.path)})</span>`),
      line('<span class="output-key">Finished</span>dev profile in 1.28s'), gap,
      command('git log --oneline -3'),
      line('<span class="terminal-warning">c7e3a21</span> Refine terminal workspace'),
      line('<span class="terminal-warning">4b19d0f</span> Add pane layout presets'),
      line('<span class="terminal-warning">8f2c6a9</span> Initialize Glim'), prompt,
    ].join('');
  }

  function announce(message) { $('announcement').textContent = message; }
  function markReview(name) {
    state.review = name;
    $('review').querySelectorAll('button').forEach((button) => button.setAttribute('aria-pressed', String(button.dataset.view === name)));
  }
  function previewTree() {
    const current = tab();
    if (state.preview !== null && current.tree) {
      const ids = leaves(current.tree), active = state.drag?.pane ?? current.active;
      return templatesFor(ids)[state.preview].build([active, ...ids.filter((id) => id !== active)]);
    }
    return current.zoomed !== null ? leaf(current.zoomed) : current.tree;
  }
  function position(element, [x, y, w, h]) {
    element.style.left = `${x * 100}%`; element.style.top = `${y * 100}%`;
    element.style.width = `${w * 100}%`; element.style.height = `${h * 100}%`;
  }
  function renderTabs() {
    for (const [id, element] of tabElements) {
      if (!state.tabs.some((item) => item.id === id)) { element.remove(); tabElements.delete(id); }
    }
    for (const item of state.tabs) {
      let element = tabElements.get(item.id);
      if (!element) {
        element = document.createElement('div'); element.className = 'tab'; element.dataset.tab = item.id;
        element.innerHTML = `<button type="button" class="tab-select" id="tab-${item.id}" role="tab" aria-controls="workspace">${icon('terminal')}<span class="tab-label">${escape(item.title)}</span></button><button type="button" class="icon-button tab-close" aria-label="关闭 ${escape(item.title)} 标签页" title="关闭标签页">${icon('close')}</button>`;
        element.querySelector('.tab-select').addEventListener('click', () => selectTab(item.id));
        element.querySelector('.tab-close').addEventListener('click', () => closeTab(item.id));
        tabElements.set(item.id, element); elements.tabs.appendChild(element);
      }
      const selected = item.id === state.activeTab, button = element.querySelector('.tab-select');
      element.classList.toggle('active', selected); button.setAttribute('aria-selected', String(selected));
      button.tabIndex = selected ? 0 : -1;
    }
    elements.workspace.setAttribute('aria-labelledby', `tab-${state.activeTab}`);
  }
  function createPane(id) {
    const info = paneInfo.get(id), running = info.kind === 'server';
    const element = document.createElement('section'); element.className = 'pane'; element.dataset.pane = id;
    element.setAttribute('aria-label', `${info.path} · ${running ? '开发服务器' : info.kind === 'tests' ? '测试' : '终端'}`);
    element.innerHTML = `<header class="pane-header" id="pane-head-${id}" tabindex="0" aria-label="${escape(info.path)}，可拖动标题交换位置"><span class="pane-title">${icon('terminal')}<span class="path" title="${escape(info.path)}">${escape(info.path)}</span></span><span class="process ${running ? 'running' : ''}">${running ? 'vite · 运行中' : info.kind === 'tests' ? '测试' : 'pwsh'}</span><span class="pane-actions"><button class="icon-button pane-split" id="pane-split-${id}" type="button" aria-label="分屏" aria-haspopup="menu" aria-controls="splitMenu" aria-expanded="false" title="分屏…">${icon('split')}</button><button class="icon-button pane-zoom" type="button" aria-label="聚焦窗格" title="聚焦窗格 · Ctrl+Shift+Enter">${icon('expand')}</button><button class="icon-button danger pane-close" type="button" aria-label="关闭窗格" title="关闭窗格 · Ctrl+Shift+W">${icon('close')}</button></span></header><div class="terminal" tabindex="0" aria-label="终端输出"><div class="terminal-content">${terminalHTML(info)}</div></div><div class="swap-indicator" hidden><span>${icon('swap')}松手交换位置</span></div>`;
    element.querySelector('.pane-header').addEventListener('pointerdown', (event) => startDrag(event, id));
    element.querySelector('.pane-split').addEventListener('click', (event) => openSplit(id, event.currentTarget));
    element.querySelector('.pane-zoom').addEventListener('click', () => zoomPane(id));
    element.querySelector('.pane-close').addEventListener('click', () => closePane(id));
    element.addEventListener('pointerdown', () => {
      if (tab().active !== id) { tab().active = id; render(); }
    });
    element.addEventListener('focusin', () => {
      if (tab().active !== id && !state.overlay && !state.resize) { tab().active = id; render(); }
    });
    elements.panes.appendChild(element); paneElements.set(id, element);
    return element;
  }
  function render() {
    const current = tab(), tree = previewTree(), layout = geometry(tree), visible = new Set(layout.panes.keys());
    renderTabs();
    for (const [id, element] of paneElements) {
      element.hidden = !visible.has(id);
      if (element.hidden) element.classList.remove('active', 'lifted', 'target', 'swap');
    }
    for (const [id, box] of layout.panes) {
      const element = paneElements.get(id) ?? createPane(id);
      element.hidden = false; position(element, box);
      element.classList.toggle('active', id === current.active);
      element.classList.toggle('lifted', !!state.drag?.moved && state.drag.pane === id && state.preview === null);
      element.classList.toggle('target', state.preview !== null && id === (state.drag?.pane ?? current.active));
      element.classList.toggle('swap', state.swapTarget === id);
      element.querySelector('.swap-indicator').hidden = state.swapTarget !== id;
      const zoomed = current.zoomed === id, zoom = element.querySelector('.pane-zoom');
      zoom.innerHTML = icon(zoomed ? 'restore' : 'expand');
      zoom.setAttribute('aria-label', zoomed ? '恢复分屏' : '聚焦窗格');
      zoom.title = `${zoomed ? '恢复分屏' : '聚焦窗格'} · Ctrl+Shift+Enter`;
      element.querySelector('.pane-split').setAttribute('aria-expanded', String(state.overlay?.kind === 'split' && state.overlay.pane === id));
    }
    renderDividers(layout);
    elements.empty.hidden = !!tree;
    elements.workspace.classList.toggle('dragging', !!state.drag?.moved);
    $('dropHint').classList.toggle('visible', !!state.drag?.moved && !state.hot && state.swapTarget === null);
    elements.picker.hidden = state.overlay?.kind !== 'layout';
    elements.menu.hidden = state.overlay?.kind !== 'split';
    $('layoutButton').disabled = !current.tree;
    $('splitButton').disabled = current.active === null;
    $('layoutButton').setAttribute('aria-expanded', String(state.overlay?.kind === 'layout'));
    $('splitButton').setAttribute('aria-expanded', String(state.overlay?.kind === 'split' && state.overlay.trigger === 'splitButton'));
    if (state.overlay?.kind === 'layout') renderTemplates();
    if (state.overlay?.kind === 'split') positionMenu();
    const active = paneInfo.get(current.active);
    $('statusPath').textContent = active?.path ?? '没有活动终端';
    $('statusShell').textContent = active?.shell ?? 'PowerShell';
    $('paneCount').textContent = `${leaves(current.tree).length} 个窗格${current.zoomed !== null ? ' · 已聚焦' : ''}`;
    // Keep session output and scroll positions when tabs or layout change, but release closed sessions.
    const live = new Set(state.tabs.flatMap((item) => leaves(item.tree)));
    for (const [id, element] of paneElements) if (!live.has(id)) { element.remove(); paneElements.delete(id); }
  }
  function renderDividers(layout) {
    for (const [id, element] of dividerElements) if (!layout.splits.has(id)) { element.remove(); dividerElements.delete(id); }
    for (const [id, { node, box }] of layout.splits) {
      if (state.preview !== null) continue;
      let element = dividerElements.get(id);
      if (!element) {
        element = document.createElement('div'); element.className = `divider ${node.axis}`; element.tabIndex = 0;
        element.setAttribute('role', 'separator'); element.setAttribute('aria-orientation', node.axis === 'row' ? 'vertical' : 'horizontal');
        element.setAttribute('aria-label', node.axis === 'row' ? '调整左右窗格宽度' : '调整上下窗格高度');
        element.setAttribute('aria-valuemin', '15'); element.setAttribute('aria-valuemax', '85');
        element.addEventListener('pointerdown', (event) => startResize(event, id));
        element.addEventListener('keydown', (event) => resizeByKey(event, id));
        dividerElements.set(id, element); elements.dividers.appendChild(element);
      }
      const [x, y, w, h] = box;
      if (node.axis === 'row') {
        element.style.left = `${(x + w * node.ratio) * 100}%`; element.style.top = `${y * 100}%`; element.style.height = `${h * 100}%`;
      } else {
        element.style.left = `${x * 100}%`; element.style.top = `${(y + h * node.ratio) * 100}%`; element.style.width = `${w * 100}%`;
      }
      element.setAttribute('aria-valuenow', String(Math.round(node.ratio * 100)));
      element.classList.toggle('resizing', state.resize?.id === id);
    }
  }
  function shape(node) {
    return node.type === 'leaf' ? 'pane' : `${node.axis}:${node.ratio.toFixed(2)}(${shape(node.a)},${shape(node.b)})`;
  }
  function renderTemplates() {
    const ids = leaves(tab().tree), options = templatesFor(ids);
    $('pickerCount').textContent = `${ids.length} 个窗格`;
    if (elements.templates.dataset.count !== String(ids.length)) {
      elements.templates.dataset.count = ids.length; elements.templates.replaceChildren();
      options.forEach((option, ix) => {
        const button = document.createElement('button'); button.type = 'button'; button.className = 'template'; button.dataset.template = ix;
        button.setAttribute('aria-label', `应用${option.name}布局`);
        const thumbnail = [...geometry(option.build(ids)).panes.values()].map(([x, y, w, h]) => `<i style="left:calc(${x * 100}% + 1px);top:calc(${y * 100}% + 1px);width:calc(${w * 100}% - 2px);height:calc(${h * 100}% - 2px)"></i>`).join('');
        button.innerHTML = `<span class="thumbnail" aria-hidden="true">${thumbnail}</span><span class="template-label">${option.name}</span>`;
        const preview = () => { state.preview = ix; render(); };
        button.addEventListener('pointerenter', preview); button.addEventListener('focus', preview);
        button.addEventListener('pointerleave', () => { if (document.activeElement !== button) { state.preview = null; render(); } });
        button.addEventListener('blur', () => { if (state.preview === ix) { state.preview = null; render(); } });
        button.addEventListener('click', () => applyTemplate(ix));
        elements.templates.appendChild(button);
      });
    }
    elements.templates.querySelectorAll('button').forEach((button, ix) => {
      button.classList.toggle('previewed', state.preview === ix);
      button.setAttribute('aria-pressed', String(shape(tab().tree) === shape(options[ix].build(ids))));
    });
  }
  function restoreFocus(trigger) {
    const target = $(trigger) ?? $(`pane-head-${tab().active}`) ?? $('tabAdd');
    target.focus({ preventScroll: true });
  }
  function closeOverlay(restore = true) {
    const trigger = state.overlay?.trigger;
    state.overlay = null; state.preview = null; render();
    if (restore && trigger) restoreFocus(trigger);
  }
  function openLayout(keyboard = false) {
    if (!tab().tree) return;
    if (state.overlay?.kind === 'layout' && !state.drag) { closeOverlay(); return; }
    state.overlay = { kind: 'layout', trigger: 'layoutButton' }; state.preview = null; render();
    if (keyboard) elements.templates.querySelector('button')?.focus();
  }
  function openSplit(pane, trigger, keyboard = false) {
    if (pane === null) return;
    const toggle = state.overlay?.kind === 'split' && state.overlay.trigger === trigger.id;
    closeOverlay(false);
    if (toggle) { restoreFocus(trigger.id); return; }
    tab().active = pane; state.overlay = { kind: 'split', pane, trigger: trigger.id }; render();
    if (keyboard) elements.menu.querySelector('button').focus();
  }
  function positionMenu() {
    const anchor = $(state.overlay.trigger), bounds = anchor?.getBoundingClientRect(), app = elements.app.getBoundingClientRect();
    if (!bounds) return;
    const width = elements.menu.offsetWidth, height = elements.menu.offsetHeight, inset = 8;
    elements.menu.style.left = `${Math.max(inset, Math.min(bounds.right - app.left - width, app.width - width - inset))}px`;
    elements.menu.style.top = `${Math.max(inset, Math.min(bounds.bottom - app.top + 4, app.height - height - inset))}px`;
  }
  function addTab() {
    closeOverlay(false); const fresh = newTab(); state.tabs.push(fresh); state.activeTab = fresh.id;
    markReview('workspace'); render(); restoreFocus(`pane-head-${fresh.active}`); announce('已新建标签页');
  }
  function selectTab(id, keyboard = false) {
    closeOverlay(false); cancelDrag(); state.activeTab = id; markReview('workspace'); render();
    if (keyboard) restoreFocus(`tab-${id}`);
  }
  function closeTab(id) {
    const ix = state.tabs.findIndex((item) => item.id === id), selected = state.activeTab === id;
    closeOverlay(false); cancelDrag(); state.tabs.splice(ix, 1);
    if (!state.tabs.length) {
      state.tabs.push({ id: nextTabId++, title: '终端', tree: null, active: null, zoomed: null });
      state.activeTab = state.tabs[0].id;
    } else if (selected) state.activeTab = state.tabs[Math.min(ix, state.tabs.length - 1)].id;
    markReview('workspace'); render(); restoreFocus(`tab-${state.activeTab}`); announce('已关闭标签页');
  }
  function splitActive(axis) {
    const current = tab(), active = state.overlay?.pane ?? current.active;
    if (active === null) return;
    const pane = newPane('shell', paneInfo.get(active).path);
    current.tree = replacePane(current.tree, active, (node) => split(axis, .5, node, leaf(pane)));
    current.active = pane; current.zoomed = null; closeOverlay(false); markReview('workspace'); render();
    restoreFocus(`pane-head-${pane}`); announce(axis === 'row' ? '已向右分屏' : '已向下分屏');
  }
  function closePane(pane) {
    const current = tab(); closeOverlay(false);
    current.tree = replacePane(current.tree, pane, () => null);
    if (current.active === pane) current.active = leaves(current.tree)[0] ?? null;
    if (current.zoomed === pane) current.zoomed = null;
    markReview('workspace'); render(); restoreFocus(current.active === null ? 'newPaneButton' : `pane-head-${current.active}`);
    announce('已关闭窗格');
  }
  function zoomPane(pane) {
    closeOverlay(false); tab().active = pane; tab().zoomed = tab().zoomed === pane ? null : pane;
    markReview('workspace'); render(); restoreFocus(`pane-head-${pane}`);
  }
  function applyTemplate(ix) {
    const current = tab(), ids = leaves(current.tree), active = state.drag?.pane ?? current.active;
    const option = templatesFor(ids)[ix]; if (!option) return;
    current.tree = option.build([active, ...ids.filter((id) => id !== active)]); current.active = active; current.zoomed = null;
    state.drag = null; state.swapTarget = null; state.hot = false; elements.ghost.hidden = true;
    closeOverlay(false); markReview('workspace'); render(); restoreFocus(`pane-head-${active}`);
    announce(`已应用${option.name}布局`);
  }

  function startDrag(event, pane) {
    if (event.button !== 0 || event.target.closest('button')) return;
    closeOverlay(false); state.swapTarget = null; state.hot = false;
    state.drag = { pane, x: event.clientX, y: event.clientY, moved: false };
    event.preventDefault();
  }
  function cancelDrag() {
    const hadDrag = !!state.drag;
    state.drag = null; state.swapTarget = null; state.hot = false; elements.ghost.hidden = true;
    if (hadDrag && state.overlay?.kind === 'layout') { state.overlay = null; state.preview = null; }
  }
  document.addEventListener('pointermove', (event) => {
    if (state.resize) { moveResize(event); return; }
    const drag = state.drag; if (!drag || drag.review) return;
    if (!drag.moved && Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 5) return;
    if (!drag.moved) { drag.moved = true; elements.ghost.innerHTML = `${icon('grip')}${escape(paneInfo.get(drag.pane).path)}`; }
    elements.ghost.hidden = false; elements.ghost.style.left = `${event.clientX + 12}px`; elements.ghost.style.top = `${event.clientY + 12}px`;
    const bounds = elements.workspace.getBoundingClientRect();
    const inside = event.clientX >= bounds.left && event.clientX <= bounds.right && event.clientY >= bounds.top && event.clientY <= bounds.bottom;
    state.hot = inside && event.clientY - bounds.top < Math.min(bounds.height * .25, 144);
    state.swapTarget = null;
    if (state.hot) {
      if (state.overlay?.kind !== 'layout') { state.overlay = { kind: 'layout', trigger: `pane-head-${drag.pane}` }; render(); }
      const card = document.elementFromPoint(event.clientX, event.clientY)?.closest('.template');
      state.preview = card ? Number(card.dataset.template) : null;
    } else {
      state.overlay = null; state.preview = null;
      if (inside) for (const [id, element] of paneElements) {
        if (id === drag.pane || element.hidden) continue;
        const box = element.getBoundingClientRect();
        if (event.clientX >= box.left && event.clientX < box.right && event.clientY >= box.top && event.clientY < box.bottom) { state.swapTarget = id; break; }
      }
    }
    render();
  });
  document.addEventListener('pointerup', (event) => {
    if (state.resize) { state.resize = null; render(); return; }
    const drag = state.drag; if (!drag || drag.review) return;
    const card = document.elementFromPoint(event.clientX, event.clientY)?.closest('.template');
    if (drag.moved && state.hot && card) { applyTemplate(Number(card.dataset.template)); return; }
    if (drag.moved && !state.hot && state.swapTarget !== null) {
      tab().tree = swapPanes(tab().tree, drag.pane, state.swapTarget); tab().active = drag.pane;
      announce('已交换窗格位置');
    } else tab().active = drag.pane;
    cancelDrag(); markReview('workspace'); render(); restoreFocus(`pane-head-${drag.pane}`);
  });
  document.addEventListener('pointercancel', () => { cancelDrag(); cancelResize(); render(); });
  window.addEventListener('blur', () => { cancelDrag(); cancelResize(); render(); });

  function resizeBounds(node, box) {
    const bounds = elements.workspace.getBoundingClientRect(), rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
    const minimum = (part, axis) => part.type === 'leaf' ? (axis === 'row' ? 12 : 8) * rem : part.axis === axis ? minimum(part.a, axis) + minimum(part.b, axis) : Math.max(minimum(part.a, axis), minimum(part.b, axis));
    const extent = node.axis === 'row' ? box[2] * bounds.width : box[3] * bounds.height;
    return [Math.min(.45, Math.max(.15, minimum(node.a, node.axis) / extent)), Math.max(.55, Math.min(.85, 1 - minimum(node.b, node.axis) / extent))];
  }
  function startResize(event, id) {
    if (event.button !== 0) return;
    const part = geometry(tab().tree).splits.get(id); if (!part) return;
    closeOverlay(false); cancelDrag(); state.resize = { id, ratio: part.node.ratio };
    event.currentTarget.focus(); event.preventDefault(); render();
  }
  function moveResize(event) {
    const part = geometry(tab().tree).splits.get(state.resize.id); if (!part) return;
    const { node, box } = part, bounds = elements.workspace.getBoundingClientRect();
    const ratio = node.axis === 'row' ? ((event.clientX - bounds.left) / bounds.width - box[0]) / box[2] : ((event.clientY - bounds.top) / bounds.height - box[1]) / box[3];
    const [min, max] = resizeBounds(node, box); node.ratio = Math.max(min, Math.min(max, ratio)); render();
  }
  function cancelResize() {
    if (!state.resize) return;
    const part = geometry(tab().tree).splits.get(state.resize.id);
    if (part) part.node.ratio = state.resize.ratio;
    state.resize = null;
  }
  function resizeByKey(event, id) {
    const part = geometry(tab().tree).splits.get(id); if (!part) return;
    const decrement = part.node.axis === 'row' ? 'ArrowLeft' : 'ArrowUp', increment = part.node.axis === 'row' ? 'ArrowRight' : 'ArrowDown';
    if (![decrement, increment, 'Home'].includes(event.key)) return;
    event.preventDefault(); event.stopPropagation();
    const [min, max] = resizeBounds(part.node, part.box);
    part.node.ratio = event.key === 'Home' ? .5 : Math.max(min, Math.min(max, part.node.ratio + (event.key === increment ? .05 : -.05)));
    render();
  }
  function directionalPane(direction) {
    const rects = geometry(tab().tree).panes, from = rects.get(tab().active); if (!from) return null;
    const cx = from[0] + from[2] / 2, cy = from[1] + from[3] / 2;
    let closest = null, distance = Infinity;
    for (const [id, box] of rects) {
      if (id === tab().active) continue;
      const dx = box[0] + box[2] / 2 - cx, dy = box[1] + box[3] / 2 - cy;
      if (direction === 'ArrowRight' && dx <= 0 || direction === 'ArrowLeft' && dx >= 0 || direction === 'ArrowDown' && dy <= 0 || direction === 'ArrowUp' && dy >= 0) continue;
      const cost = ['ArrowRight', 'ArrowLeft'].includes(direction) ? Math.abs(dx) + 2 * Math.abs(dy) : Math.abs(dy) + 2 * Math.abs(dx);
      if (cost < distance) { closest = id; distance = cost; }
    }
    return closest;
  }
  function navigateOverlay(event) {
    const root = state.overlay?.kind === 'split' ? elements.menu : elements.templates;
    const buttons = [...root.querySelectorAll('button')], ix = buttons.indexOf(document.activeElement);
    if (event.key === 'Tab') { closeOverlay(); return; }
    if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    event.preventDefault();
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (ix + (['ArrowRight', 'ArrowDown'].includes(event.key) ? 1 : -1) + buttons.length) % buttons.length;
    buttons[next]?.focus();
  }
  document.addEventListener('keydown', (event) => {
    const key = event.key.toLowerCase();
    if (key === 'escape') {
      event.preventDefault();
      if (state.resize) cancelResize();
      else if (state.drag) cancelDrag();
      else if (state.overlay) closeOverlay();
      else if (tab().zoomed !== null) zoomPane(tab().zoomed);
      render(); return;
    }
    if (state.overlay && !event.ctrlKey && !event.altKey) { navigateOverlay(event); return; }
    if (event.altKey && key === 'z') { event.preventDefault(); openLayout(true); }
    else if (event.ctrlKey && event.shiftKey && key === 't') { event.preventDefault(); addTab(); }
    else if (event.ctrlKey && event.shiftKey && key === 'w') { event.preventDefault(); if (tab().active !== null) closePane(tab().active); }
    else if (event.ctrlKey && event.shiftKey && ['d', 'e'].includes(key)) { event.preventDefault(); splitActive(key === 'd' ? 'row' : 'col'); }
    else if (event.ctrlKey && event.shiftKey && key === 'enter') { event.preventDefault(); if (tab().active !== null) zoomPane(tab().active); }
    else if (event.ctrlKey && key === 'tab') {
      event.preventDefault(); const ix = state.tabs.findIndex((item) => item.id === state.activeTab);
      selectTab(state.tabs[(ix + (event.shiftKey ? -1 : 1) + state.tabs.length) % state.tabs.length].id, true);
    } else if (event.target.matches('.tab-select') && ['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
      event.preventDefault(); const ix = state.tabs.findIndex((item) => item.id === state.activeTab);
      const next = key === 'home' ? 0 : key === 'end' ? state.tabs.length - 1 : (ix + (key === 'arrowright' ? 1 : -1) + state.tabs.length) % state.tabs.length;
      selectTab(state.tabs[next].id, true);
    } else if (event.altKey && event.key.startsWith('Arrow') || event.ctrlKey && event.shiftKey && event.key.startsWith('Arrow')) {
      event.preventDefault(); const target = directionalPane(event.key); if (target === null) return;
      if (event.ctrlKey && event.shiftKey) { tab().tree = swapPanes(tab().tree, tab().active, target); announce('已交换窗格位置'); }
      else { tab().active = target; if (tab().zoomed !== null) tab().zoomed = target; }
      markReview('workspace'); render(); restoreFocus(`pane-head-${tab().active}`);
    }
  });

  document.addEventListener('pointerdown', (event) => {
    if (state.overlay && !event.target.closest('.overlay') && !event.target.closest('[aria-haspopup]')) closeOverlay(false);
  });
  $('tabAdd').addEventListener('click', addTab);
  $('newPaneButton').addEventListener('click', () => {
    const pane = newPane(); tab().tree = leaf(pane); tab().active = pane;
    markReview('workspace'); render(); restoreFocus(`pane-head-${pane}`);
  });
  $('layoutButton').addEventListener('click', (event) => openLayout(event.detail === 0));
  $('pickerClose').addEventListener('click', () => closeOverlay());
  $('splitButton').addEventListener('click', (event) => openSplit(tab().active, event.currentTarget, event.detail === 0));
  elements.menu.querySelectorAll('button').forEach((button) => button.addEventListener('click', () => splitActive(button.dataset.axis)));

  const media = matchMedia('(prefers-color-scheme: dark)');
  const validThemes = ['dark', 'light', 'system'];
  let theme = 'dark';
  try { const saved = localStorage.getItem('glim.design.theme'); if (validThemes.includes(saved)) theme = saved; } catch {}
  const hash = location.hash.slice(1); if (validThemes.includes(hash)) theme = hash;
  function applyTheme() {
    document.documentElement.dataset.theme = theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
    $('themeSwitch').querySelectorAll('button').forEach((button) => button.setAttribute('aria-pressed', String(button.dataset.theme === theme)));
  }
  $('themeSwitch').addEventListener('click', (event) => {
    const button = event.target.closest('button'); if (!button) return;
    theme = button.dataset.theme; try { localStorage.setItem('glim.design.theme', theme); } catch {}
    applyTheme();
  });
  media.addEventListener('change', () => { if (theme === 'system') applyTheme(); });
  window.addEventListener('resize', () => { if (state.overlay?.kind === 'split') positionMenu(); });

  function reviewView(name) {
    closeOverlay(false); cancelDrag(); cancelResize();
    state.tabs = JSON.parse(baseline); state.activeTab = state.tabs[0].id;
    markReview(name); render();
    if (name === 'layout') openLayout();
    if (name === 'menu') openSplit(tab().active, $(`pane-split-${tab().active}`));
    if (name === 'empty') { tab().tree = null; tab().active = null; render(); }
    if (name === 'swap') {
      state.drag = { pane: tab().active, moved: true, review: true };
      state.swapTarget = leaves(tab().tree)[1]; render();
    }
  }
  $('review').addEventListener('click', (event) => {
    const button = event.target.closest('button'); if (button) reviewView(button.dataset.view);
  });
  render(); applyTheme();
  const requestedView = new URLSearchParams(location.search).get('view');
  if (['layout', 'menu', 'swap', 'empty'].includes(requestedView)) reviewView(requestedView);
})();
