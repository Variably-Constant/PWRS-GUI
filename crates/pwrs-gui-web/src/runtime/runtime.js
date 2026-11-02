// PWRS-GUI's widget runtime: draws a window's tree, applies the session's
// patches and table rows, and posts what people do as the egui renderer
// reports it: Click with no value, Select with the sorted selected row
// numbers, a tree's selected items in tree order, or a chart's point,
// Activate with the row or item double-clicked or chosen with Enter,
// Change with an input's new value, Submit with a field's text, Shortcut
// on the root with one of the window's own shortcuts, Expand on a tree with
// an item opened whose children are still to come, Drop with the paths of
// files or the text dropped on a widget, and PointerDown, PointerUp,
// PointerMove and Wheel with where on a GPU surface the pointer is, which
// the page also draws with WebGPU. Keys pressed go to the helper,
// which raises Key for those the focused widget does not use itself, and
// presses the window's default button for Enter and its cancel button for
// Escape where that widget does not use them. A window fitting its content
// is measured before the helper shows it. Every
// control is drawn here as the egui renderer draws it, its popups too, and
// none of the engine's own controls shows.
(function () {
  'use strict';
  var post = window.pwrs.__post;
  var root = document.getElementById('root');
  var tree = null;
  // Whether a button bar puts the default button first, as Windows orders
  // a dialog's buttons, and draws its strip, as the helper's system does.
  var windowsOrder = false;
  // Whether the window opened as a dialog, drawn inside a dialog's margin
  // with or without a button bar, as the ready-made windows are.
  var dialogWindow = false;
  var lists = {};
  var tables = {};
  // Each tree's selected items, the item its Shift ranges run from, and the
  // item its keyboard is on, by node id.
  var trees = {};
  // The tree items a person has open, and those whose Expand was raised and
  // whose children the session has not sent yet, in every tree.
  var openItems = new Set();
  var expanding = new Set();
  // Each check box's, text field's, number field's, slider's, drop-down's,
  // option group's and set of tabs' value: true or false, the text, the
  // number, or the row chosen, null for none.
  var values = {};
  // Each password field's peek rule (pwrsLayout.peek), and the fields whose
  // peek button a test holds, by node id.
  var peeks = {};
  var peekHeld = {};
  // Each password field's function that shows or hides its password as its
  // peek button, Alt+F8 and a test's hold say, by node id.
  var peekShows = {};
  // Each password field's function that gives its peek rule the field's
  // focus and text now, by node id. A window without the system's focus
  // gets no blur when its focus moves, so a focus request and a snapshot
  // run them too, as egui gives the rule every frame.
  var peekSyncs = {};
  // Each split's share of its space given to its first child, from 0 to 1.
  var splits = {};
  // Whether each log stays scrolled to its newest line: a person
  // scrolling up turns it off, and scrolling back to the end on again.
  var logEnds = {};
  // The containers the page scrolls, which drawing again leaves where they
  // were scrolled to: the window's body, a list, tree, log or scroll area, a
  // table's rows, a split's panes and a block of code.
  var SCROLLERS = '#root > .body, .list, .tree, .log, .scroll-area, .table > .scroll, .split > .pane, .markdown pre.code';
  var inputKinds = ['CheckBox', 'TextBox', 'NumberBox', 'Slider', 'ComboBox', 'RadioGroup', 'Tabs', 'ColorPicker', 'DatePicker', 'FilePicker'];
  // Each chart drawn, by node id: its uPlot and the observer of its size,
  // which drawing the chart again replaces.
  var charts = {};
  // The file choosers whose dialog is open, waiting on the helper.
  var browsing = new Set();
  // The menus open, outermost first, each { element, node, opener }: the
  // menu on the page, the Menu or ContextMenu it shows, and the menu bar
  // title or entry that opened it, if one did.
  var openMenus = [];
  var popups = document.createElement('div');
  document.body.appendChild(popups);
  // The popup open under a widget, if one is, { element, anchor, node }: a
  // drop-down's choices, a date chooser's calendar or a color chooser's
  // picker, which a press elsewhere, Escape, a choice or drawing its widget
  // again closes.
  var popup = null;
  // The month each date chooser's calendar shows, [year, month]: the
  // chosen day's until a person moves it.
  var calendarMonths = {};
  // Each color chooser's color as its picker last had it, { hex, h, s, v }
  // with the value in gamma, so a gray still has the hue it was given.
  var pickerColors = {};
  // Whether pickers show a color's channels as bytes or as linear
  // fractions, which a picker's first button switches for every picker.
  var channelBytes = true;
  // Text laid out as a drag value lays it out, to measure how wide it is.
  var measurer = document.createElement('span');
  measurer.style.cssText = 'position: absolute; visibility: hidden; white-space: pre; font-family: var(--font-ui); font-size: var(--button); left: -10000px; top: 0;';
  document.body.appendChild(measurer);
  // Ctrl in a shortcut stands for the Command key on macOS.
  var mac = /Mac|iPhone|iPad/.test(navigator.platform);
  // The system's line ending, which ends each row a table copies: CR LF on
  // Windows and LF elsewhere, as .NET's Environment.NewLine is.
  var NEWLINE = /Win/.test(navigator.platform) ? '\r\n' : '\n';
  // The text the window last put on the clipboard, which a snapshot
  // reports; null until it has put any there.
  var lastCopied = null;
  // The keys a shortcut names besides letters, digits and F keys, and the
  // ones a focused text field keeps with Ctrl pressed, as the protocol lists
  // them.
  var shortcutKeys = [];
  var fieldCtrlKeys = [];
  var textTypes = ['text', 'password', 'number', 'search', 'email', 'url', 'tel'];

  function props(node) { return node.props || {}; }
  function sorted(set) { return Array.from(set).sort(function (a, b) { return a - b; }); }
  function emit(node, kind, value) { post({ t: 'event', node: node.id, kind: kind, value: value === undefined ? null : value }); }
  function find(node, id) {
    if (node.id === id) return node;
    for (var i = 0; i < (node.children || []).length; i++) { var f = find(node.children[i], id); if (f) return f; }
    return null;
  }
  function findNamed(node, name) {
    if (node.name && node.name.toLowerCase() === name.toLowerCase()) return node;
    for (var i = 0; i < (node.children || []).length; i++) { var f = findNamed(node.children[i], name); if (f) return f; }
    return null;
  }
  function rowsOf(v) {
    var rows = Array.isArray(v) ? v : [v];
    rows.forEach(function (r) { if (!Number.isInteger(r) || r < 0) throw new Error(JSON.stringify(r) + ' is not a row number'); });
    return Array.from(new Set(rows)).sort(function (a, b) { return a - b; });
  }

  // A table's state as the page stores it: its columns, its rows and those
  // shown, its filter, sort, selection and anchor, the width a person
  // dragged each column to, what places its column lines again, and whether
  // Add-GuiItem added rows since it was last laid out and whether that
  // layout showed its last row, which fit reads to bring the last row into
  // sight.
  function newTable() {
    return { columns: [], rows: [], view: [], filter: '', sort: null, selected: new Set(), anchor: null, widths: [], edges: null, dirty: true, added: false, end: false };
  }

  // Sets up the input state of a subtree's widgets from their properties;
  // with `set`, one node's from the properties a patch set, leaving what a
  // person changed in the rest as it is. New items set without a selection
  // leave the chosen rows that are still among them chosen.
  function seed(node, set) {
    var p = props(node);
    var given = function (key) { return !set || set.indexOf(key) >= 0; };
    var count = Array.isArray(p.items) ? p.items.length : 0;
    if (node.kind === 'ListBox') {
      if (given('selected')) lists[node.id] = { selected: new Set(p.selected === undefined ? [] : rowsOf(p.selected)) };
      else if (given('items') && lists[node.id]) lists[node.id].selected = new Set(sorted(lists[node.id].selected).filter(function (r) { return r < count; }));
    } else if (node.kind === 'Table') {
      var t = tables[node.id] || (tables[node.id] = newTable());
      if (given('columns')) {
        t.columns = (p.columns || []).map(function (c) {
          return { label: c && c.label !== undefined ? String(c.label) : '', align: c && c.align !== undefined && c.align !== null ? c.align : null };
        });
        t.widths = [];
        t.dirty = true;
      }
    } else if (node.kind === 'CheckBox') {
      if (given('checked')) values[node.id] = p.checked === true;
    } else if (node.kind === 'TextBox') {
      if (given('text')) values[node.id] = p.text;
      if (given('text') && p.password === true) {
        peeks[node.id] = pwrsLayout.peek(typeof p.text === 'string' ? p.text : '');
        delete peekHeld[node.id];
      }
    } else if (node.kind === 'NumberBox' || node.kind === 'Slider') {
      if (given('value')) values[node.id] = p.value;
    } else if (node.kind === 'ComboBox' && p.editable === true) {
      if (given('text')) values[node.id] = p.text;
    } else if (node.kind === 'ColorPicker') {
      if (given('color')) values[node.id] = typeof p.color === 'string' ? p.color.toLowerCase() : p.color;
    } else if (node.kind === 'DatePicker') {
      if (given('value')) values[node.id] = p.value === undefined ? null : p.value;
    } else if (node.kind === 'FilePicker') {
      if (given('paths')) values[node.id] = p.paths === undefined || p.paths === null ? [] : p.paths;
    } else if (node.kind === 'ComboBox' || node.kind === 'RadioGroup' || node.kind === 'Tabs') {
      var total = node.kind === 'Tabs' ? (node.children || []).length : count;
      var none = node.kind === 'Tabs' && total > 0 ? 0 : null;
      if (given('selected')) values[node.id] = p.selected === undefined || p.selected === null ? none : p.selected;
      else if (given('items') && typeof values[node.id] === 'number' && values[node.id] >= total) values[node.id] = none;
    } else if (node.kind === 'Split') {
      if (given('position')) splits[node.id] = p.position === undefined ? 0.5 : p.position;
    } else if (node.kind === 'TreeView') {
      var tv = trees[node.id] || (trees[node.id] = { selected: new Set(), anchor: null, focus: null });
      if (given('selected')) tv.selected = new Set(p.selected === undefined || p.selected === null ? [] : itemsOf(p.selected));
    } else if (node.kind === 'TreeItem') {
      // The session's reply to an Expand: the item stops waiting, and one
      // given more and no children closes again, so opening it raises Expand
      // again.
      if (given('more')) {
        expanding.delete(node.id);
        if (p.more === true && !(node.children || []).length) openItems.delete(node.id);
      }
    }
    if (!set) (node.children || []).forEach(function (c) { seed(c); });
  }

  // A tree's selected items from a property: one item's node id or a list.
  function itemsOf(v) {
    var ids = Array.isArray(v) ? v : [v];
    ids.forEach(function (i) { if (!Number.isInteger(i) || i < 0) throw new Error(JSON.stringify(i) + " is not a tree item's node"); });
    return ids;
  }

  // An input's state as the renderer stores it for a page loaded again: a
  // list's selected rows, a table's selection, filter, sort and anchor, a
  // tree's selection, anchor, keyboard and open items, a split's position,
  // any other input's value.
  function stateOf(id) {
    if (lists[id]) return sorted(lists[id].selected);
    var t = tables[id];
    if (t) return { selected: sorted(t.selected), filter: t.filter, sort: t.sort, anchor: t.anchor };
    var tv = trees[id];
    if (tv) {
      var n = find(tree, Number(id));
      var open = n ? treeOrder(n.children, []).filter(function (i) { return openItems.has(i); }) : [];
      return { tree: { selected: sorted(tv.selected), anchor: tv.anchor, focus: tv.focus, open: open } };
    }
    if (id in values) return { value: values[id] };
    if (id in splits) return { position: splits[id] };
    return null;
  }
  function report(id) {
    var value = stateOf(id);
    if (value !== null) post({ t: 'state', node: Number(id), value: value });
  }
  function reportAll() {
    Object.keys(lists).concat(Object.keys(tables), Object.keys(trees), Object.keys(values), Object.keys(splits)).forEach(function (id) { if (find(tree, Number(id))) report(id); });
  }
  function restore(state) {
    Object.keys(state || {}).forEach(function (id) {
      var v = state[id];
      if (trees[id] && v && v.tree) {
        var tv = trees[id];
        tv.selected = new Set(Array.isArray(v.tree.selected) ? v.tree.selected : []);
        tv.anchor = typeof v.tree.anchor === 'number' ? v.tree.anchor : null;
        tv.focus = typeof v.tree.focus === 'number' ? v.tree.focus : null;
        (Array.isArray(v.tree.open) ? v.tree.open : []).forEach(function (i) { openItems.add(i); });
      } else if (lists[id] && Array.isArray(v)) {
        lists[id].selected = new Set(v);
      } else if (tables[id] && v && typeof v === 'object') {
        var t = tables[id];
        t.selected = new Set(Array.isArray(v.selected) ? v.selected : []);
        t.filter = typeof v.filter === 'string' ? v.filter : '';
        t.sort = Array.isArray(v.sort) ? v.sort : null;
        t.anchor = typeof v.anchor === 'number' ? v.anchor : null;
        t.dirty = true;
      } else if (id in values && v && typeof v === 'object' && 'value' in v) {
        values[id] = v.value;
      } else if (id in splits && v && typeof v.position === 'number') {
        splits[id] = v.position;
      }
    });
  }

  function compareKeys(a, b) {
    var na = typeof a === 'number', nb = typeof b === 'number';
    var ea = a === null || a === undefined, eb = b === null || b === undefined;
    if (na && nb) return a - b;
    if (typeof a === 'string' && typeof b === 'string') { var x = a.toLowerCase(), y = b.toLowerCase(); return x < y ? -1 : x > y ? 1 : 0; }
    if (ea && eb) return 0;
    if (ea) return 1;
    if (eb) return -1;
    return na ? -1 : 1;
  }

  function refresh(t) {
    if (!t.dirty) return;
    t.dirty = false;
    var needle = t.filter.toLowerCase();
    t.view = [];
    for (var r = 0; r < t.rows.length; r++) {
      if (!needle || t.rows[r].some(function (c) { return c.text.toLowerCase().indexOf(needle) >= 0; })) t.view.push(r);
    }
    if (t.sort) {
      var col = t.sort[0], up = t.sort[1];
      t.view.sort(function (a, b) {
        var ca = t.rows[a][col], cb = t.rows[b][col];
        var o = compareKeys(ca ? ca.sort : null, cb ? cb.sort : null);
        return up ? o : -o;
      });
    }
  }

  // The focused element's node and caret, which drawing again puts back.
  function focusOf() {
    var a = document.activeElement;
    if (!a || !a.dataset || !a.dataset.node) return null;
    return { node: a.dataset.node, caret: typeof a.selectionStart === 'number' ? [a.selectionStart, a.selectionEnd] : null };
  }
  function refocus(keep) {
    if (!keep) return;
    var again = root.querySelector('[data-node="' + keep.node + '"]');
    if (!again) return;
    again.focus();
    if (keep.caret && typeof again.selectionStart === 'number') again.setSelectionRange(keep.caret[0], keep.caret[1]);
  }
  // Draws the window: its menu bars along the top, its status bars along
  // the bottom and its button bar above them, whatever their place among
  // the root's children, and its other widgets between them, inside a
  // dialog's margin when it has a button bar or opened as a dialog. Drawing
  // again closes any menu open.
  function render() {
    var keep = focusOf();
    closeMenus(0);
    closePopup();
    var bars = ownOf('MenuBar'), status = ownOf('StatusBar'), buttons = ownOf('ButtonBar')[0] || null;
    root.classList.toggle('dialog', buttons !== null || dialogWindow);
    var parts = [];
    if (bars.length) {
      var top = el('div', 'menubars');
      bars.forEach(function (b) { top.appendChild(drawMenuBar(b)); });
      parts.push(top);
    }
    var middle = el('div', 'body');
    middle.appendChild(draw(tree));
    parts.push(middle);
    if (buttons) parts.push(drawButtonBar(buttons));
    if (status.length) {
      var bottom = el('div', 'statusbars');
      status.forEach(function (b) { bottom.appendChild(drawStatusBar(b)); });
      parts.push(bottom);
    }
    var scrolled = scrollsIn(root);
    root.replaceChildren.apply(root, parts);
    scrollBack(root, scrolled);
    refocus(keep);
    fit();
  }
  // Draws one node again in its place, so the rest of the window, such as a
  // slider being dragged, stays as it is, and its scroll containers where
  // they were scrolled to; the whole window when the node is not drawn, or
  // is one render draws.
  function redraw(node) {
    if (chrome(node.id)) { render(); return; }
    var old = root.querySelector('[data-node="' + node.id + '"]');
    if (!old) { render(); return; }
    var keep = focusOf();
    var scrolled = scrollsIn(old);
    var fresh = draw(node);
    old.replaceWith(fresh);
    if (fresh.parentElement && fresh.parentElement.classList.contains('grid')) markColumns(fresh.parentElement);
    scrollBack(fresh, scrolled);
    if (popup && !popup.anchor.isConnected) closePopup();
    refocus(keep);
    fit();
  }
  // The scroll containers in `within`, itself among them, each keyed by its
  // widget's node, its first class and its place among that widget's
  // containers of that class, which the same widget drawn again gives.
  function scrollers(within) {
    var found = {}, counts = {};
    var all = Array.prototype.slice.call(within.querySelectorAll(SCROLLERS));
    if (within.matches(SCROLLERS)) all.unshift(within);
    all.forEach(function (e) {
      var owner = e.closest('[data-node]');
      var key = (owner ? owner.dataset.node : '') + ' ' + e.classList[0];
      counts[key] = (counts[key] || 0) + 1;
      found[key + ' ' + counts[key]] = e;
    });
    return found;
  }
  // Where each scroll container in `within` that is scrolled is scrolled to.
  function scrollsIn(within) {
    var kept = {}, found = scrollers(within);
    Object.keys(found).forEach(function (k) {
      var e = found[k];
      if (e.scrollTop || e.scrollLeft) kept[k] = [e.scrollLeft, e.scrollTop];
    });
    return kept;
  }
  // Scrolls the containers in `within` to where `kept` says theirs were.
  function scrollBack(within, kept) {
    var found = scrollers(within);
    Object.keys(kept).forEach(function (k) {
      if (!found[k]) return;
      found[k].scrollLeft = kept[k][0];
      found[k].scrollTop = kept[k][1];
    });
  }
  // Scrolls each log to its newest line unless a person has scrolled it up,
  // and each table to its last row when Add-GuiItem added rows while its
  // last layout showed that row, and makes each text field of several lines
  // as tall as its text. A window being measured lays out every row, so its
  // tables wait until it is not. The widgets that fill down share their
  // Column's height through the stylesheet.
  function fit() {
    root.querySelectorAll('.log').forEach(function (e) { if (logEnds[e.dataset.node] !== false) e.scrollTop = e.scrollHeight; });
    if (!document.documentElement.classList.contains('measuring')) {
      root.querySelectorAll('.table > .scroll').forEach(function (e) {
        var t = tables[parseInt(e.dataset.node, 10)];
        if (!t) return;
        if (t.end && t.added) e.scrollTop = e.scrollHeight;
        t.added = false;
        t.end = e.scrollTop + e.clientHeight >= e.scrollHeight - 2;
      });
    }
    root.querySelectorAll('textarea.field').forEach(grow);
  }
  // A text field of several lines, as tall as its text once that passes the
  // rows it shows at least.
  function grow(field) {
    field.style.height = '';
    if (field.scrollHeight > field.clientHeight) field.style.height = (field.scrollHeight + field.offsetHeight - field.clientHeight) + 'px';
  }

  function el(tag, cls, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }
  function svgEl(tag, attributes) {
    var e = document.createElementNS('http://www.w3.org/2000/svg', tag);
    Object.keys(attributes || {}).forEach(function (k) { e.setAttribute(k, String(attributes[k])); });
    return e;
  }
  // A spinner as egui draws one: an arc of the strong text's color, turning.
  function spinner() {
    var s = svgEl('svg', { 'class': 'spinner', viewBox: '0 0 18 18' });
    s.appendChild(svgEl('circle', { cx: 9, cy: 9, r: 7 }));
    return s;
  }

  // Whether a widget takes the room across its column, as egui's scroll
  // areas, tables, progress bars, charts, splits and tabs do, a log, a GPU
  // surface and a canvas without a width, and a Markdown document with a
  // code block or a rule, which egui draws the room's width; a Column or
  // Row placing its widgets other than at the start, which needs the room
  // to place them in, a Grid stretching its last column, or one containing
  // a widget that takes the room; a Row with a Spacer that fills, which
  // takes the room the others leave; a separator does between stacked
  // widgets, not in a row.
  function wide(node) {
    var kids = node.children || [];
    var align = props(node).align;
    var placed = align !== undefined && align !== null && align !== 'start';
    switch (node.kind) {
      case 'ListBox': case 'TreeView': case 'Scroll': case 'Table': case 'ProgressBar': case 'Chart': case 'Split': case 'Tabs': return true;
      case 'LogView': case 'GpuSurface': case 'Canvas': return props(node).width === undefined || props(node).width === null;
      case 'Markdown': { var doc = props(node).document; return !!doc && typeof doc.html === 'string' && /<pre class="code">|<hr\b/.test(doc.html); }
      case 'Row': return placed || kids.some(function (c) { return (c.kind === 'Spacer' && props(c).fill === true) || (c.kind !== 'Separator' && wide(c)); });
      case 'Grid': return align === 'stretch';
      case 'Column': case 'Tab': return placed || kids.some(function (c) { return c.kind === 'Separator' || wide(c); });
      case 'Group': return kids.some(function (c) { return c.kind === 'Separator' || wide(c); });
      default: return false;
    }
  }

  // A number as egui writes it: with the fewest decimals from `least` to
  // `most` that give it back, else `most`.
  function numberText(v, least, most) {
    for (var d = least; d < most; d++) {
      var t = v.toFixed(d), back = Number(t);
      var big = Math.max(Math.abs(back), Math.abs(v));
      if (back === v || big <= 16 * 1.1920929e-7 || Math.abs(back - v) / big <= 16 * 1.1920929e-7) return t;
    }
    return v.toFixed(most);
  }
  // The decimals egui gives a drag value moving `speed` a point: as many as
  // a physical pixel's worth of `speed` shows.
  function decimalsFor(speed) {
    var d = Math.ceil(Math.log10((1 / (window.devicePixelRatio || 1)) / Math.abs(speed)));
    return Math.min(Math.max(isFinite(d) ? d : 0, 0), 15);
  }
  // A number as egui's drag value shows it: a button with the number, as
  // wide as it and a control wide at least, dragged across or up to change
  // it by `speed` a point; clicked or tabbed to, a field to type a number
  // in, which Enter or leaving takes and Escape drops, Up and Down stepping
  // it. `o` has the value, its range, speed, a prefix shown before it, a
  // writer for it, whether it is enabled, whether it takes a text field's
  // look (`well`), the node id it is found by, and `set`, which takes a new
  // value and returns the value it stored. Its own width is also `--own-w`,
  // which a stretched layout's rules count, and the engine's width for a
  // field of one character is the least it takes, so no default width
  // counts.
  function dragValue(o) {
    var f = el('input', 'btn frame drag' + (o.well ? ' well' : ''));
    f.type = 'text';
    f.size = 1;
    f.spellcheck = false;
    f.autocomplete = 'off';
    f.setAttribute('role', 'spinbutton');
    f.disabled = !o.enabled;
    if (o.node) f.dataset.node = o.node;
    var lo = o.range[0], hi = o.range[1];
    if (isFinite(lo)) f.setAttribute('aria-valuemin', String(lo));
    if (isFinite(hi)) f.setAttribute('aria-valuemax', String(hi));
    var decimals = o.decimals !== undefined ? o.decimals : decimalsFor(o.speed);
    var write = o.format || function (v) { return numberText(v, decimals, o.decimals !== undefined ? decimals : decimals + 2); };
    var value = o.value, dropping = false, drag = null;
    var show = function () {
      var typing = document.activeElement === f;
      f.value = typing ? write(value) : (o.prefix || '') + write(value);
      f.setAttribute('aria-valuenow', String(value));
      f.classList.toggle('lowest', value <= lo);
      f.classList.toggle('highest', value >= hi);
      if (!typing) {
        measurer.textContent = f.value;
        var own = Math.max(measurer.getBoundingClientRect().width + 2 * cssPoints('--button-px'), cssPoints('--interact-w')) + 'px';
        f.style.width = own;
        f.style.setProperty('--own-w', own);
      }
    };
    var take = function (v, rounded) {
      v = Math.min(Math.max(v, lo), hi);
      if (rounded) v = Number(v.toFixed(decimals));
      var kept = o.set(v);
      value = kept === undefined ? v : kept;
      show();
    };
    f.addEventListener('pointerdown', function (e) {
      if (f.disabled || e.button !== 0 || document.activeElement === f) return;
      e.preventDefault();
      drag = { x: e.clientX, y: e.clientY, from: value, moved: false };
      f.setPointerCapture(e.pointerId);
      f.classList.add('pressed');
    });
    f.addEventListener('pointermove', function (e) {
      if (!drag) return;
      var dx = e.clientX - drag.x, dy = e.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) <= 6) return;
      drag.moved = true;
      take(drag.from + (dx - dy) * o.speed, true);
    });
    var release = function () {
      if (!drag) return;
      var d = drag;
      drag = null;
      f.classList.remove('pressed');
      if (!d.moved) { f.focus(); f.select(); }
    };
    f.addEventListener('pointerup', release);
    f.addEventListener('pointercancel', release);
    f.addEventListener('focus', function () { show(); f.select(); });
    f.addEventListener('blur', function () {
      if (dropping) dropping = false;
      else {
        var typed = Number(f.value.trim().replace('−', '-'));
        if (f.value.trim() !== '' && isFinite(typed)) take(typed, false);
      }
      show();
    });
    f.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { f.blur(); e.preventDefault(); }
      else if (e.key === 'Escape') { dropping = true; f.blur(); e.preventDefault(); e.stopPropagation(); }
      else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') { take(value + (e.key === 'ArrowUp' ? o.speed : -o.speed), true); f.select(); e.preventDefault(); }
    });
    f.setValue = function (v) { value = v; show(); };
    show();
    return f;
  }
  // A length the stylesheet gives in points, by its variable.
  function cssPoints(name) { return parseFloat(getComputedStyle(document.documentElement).getPropertyValue(name)) || 0; }
  // Lays each size of text out in rows as tall as egui's: the height of a
  // row in its face, which the stylesheet gives as --row-…, rounded to a
  // whole physical pixel as egui rounds a row, in --line-…; again, and the
  // window fitted and its GPU surfaces drawn at their new size in pixels,
  // when it moves to a screen of another pixel density. A size with no row
  // height in the stylesheet has the engine's own.
  function layRows() {
    var ratio = window.devicePixelRatio || 1;
    ['small', 'body', 'button', 'heading', 'heading-2', 'heading-3', 'heading-4', 'heading-5', 'heading-6', 'instruction', 'mono'].forEach(function (size) {
      var row = cssPoints('--row-' + size);
      if (row > 0) document.documentElement.style.setProperty('--line-' + size, Math.round(row * ratio) / ratio + 'px');
    });
    matchMedia('(resolution: ' + ratio + 'dppx)').addEventListener('change', function () {
      layRows();
      fit();
      Object.keys(surfaces).forEach(function (id) { surfaces[id].dirty = true; });
      Object.keys(canvases).forEach(function (id) { canvases[id].dirty = true; });
      schedule();
    }, { once: true });
  }
  // A canvas's items left to the theme are drawn in its text color, which
  // changes with the system's light or dark preference.
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function () {
    Object.keys(canvases).forEach(function (id) { canvases[id].dirty = true; });
    schedule();
  });

  // Closes the popup open, if one is.
  function closePopup() {
    if (!popup) return;
    var p = popup;
    popup = null;
    p.element.remove();
    p.anchor.classList.remove('open');
    p.anchor.setAttribute('aria-expanded', 'false');
  }
  // Opens `element` as the popup of widget `node` under `anchor`, its left
  // edge at the anchor's, above it where the window has no room below, and
  // with `asWide` at least as wide as the anchor; inside the window.
  function openPopup(anchor, element, node, asWide) {
    closePopup();
    closeMenus(0);
    popups.appendChild(element);
    var a = anchor.getBoundingClientRect();
    if (asWide) element.style.minWidth = a.width + 'px';
    var r = element.getBoundingClientRect();
    var top = a.bottom + r.height > window.innerHeight && a.top - r.height >= 0 ? a.top - r.height : a.bottom;
    element.style.left = Math.max(0, Math.min(a.left, window.innerWidth - r.width)) + 'px';
    element.style.top = Math.max(0, top) + 'px';
    anchor.classList.add('open');
    anchor.setAttribute('aria-expanded', 'true');
    popup = { element: element, anchor: anchor, node: node };
  }

  function draw(node) {
    // A menu bar, a status bar, a button bar and a context menu take no
    // place among the widgets (pwrsLayout.inFlow): render draws the bars,
    // and a right-click opens the menu.
    if (!pwrsLayout.inFlow(node)) {
      var holder = el('span');
      holder.hidden = true;
      return holder;
    }
    var e;
    try { e = drawNode(node); } catch (err) {
      e = problemOf(node, err.message);
    }
    if (!e.dataset.node) e.dataset.node = node.id;
    hook(e, node);
    if (wide(node)) e.classList.add('wide');
    if (pwrsLayout.fillsDown(node)) e.classList.add('fill');
    return e;
  }
  // Marks element `e`, which draws widget `node`, with the hooks a window's
  // own CSS selects widgets by: the widget's kind, and the name PowerShell
  // gave it.
  function hook(e, node) {
    e.dataset.kind = node.kind;
    if (node.name) e.dataset.name = node.name;
  }
  // A widget that cannot be drawn: its problem, reported and shown in its
  // place.
  function problemOf(node, why) {
    var label = node.kind + ' ' + (node.name || node.id) + ': ' + why;
    post({ t: 'error', text: label });
    return el('div', 'label error', label);
  }

  function need(p, key, type) {
    if (p[key] === undefined) throw new Error(key + ' is required');
    if (typeof p[key] !== type) throw new Error(key + ' must be a ' + type);
    return p[key];
  }
  function flag(p, key, absent) {
    if (p[key] === undefined) return absent;
    if (typeof p[key] !== 'boolean') throw new Error(key + ' must be true or false');
    return p[key];
  }
  function optNumber(p, key) {
    var v = p[key];
    if (v === undefined || v === null) return null;
    if (typeof v !== 'number' || !isFinite(v)) throw new Error(key + ' must be a finite number');
    return v;
  }
  function needNumber(p, key) {
    var v = optNumber(p, key);
    if (v === null) throw new Error(key + ' is required');
    return v;
  }
  function needStrings(p, key) {
    if (!Array.isArray(p[key]) || p[key].some(function (i) { return typeof i !== 'string'; })) throw new Error(key + ' must be a list of strings');
    return p[key];
  }
  // A number field's or slider's range: a slider's minimum and maximum,
  // which it must have, or a number field's, unbounded where it lacks them.
  function bounds(node, p) {
    var slider = node.kind === 'Slider';
    var lo = slider ? needNumber(p, 'minimum') : optNumber(p, 'minimum');
    var hi = slider ? needNumber(p, 'maximum') : optNumber(p, 'maximum');
    if (lo !== null && hi !== null && lo > hi) throw new Error('minimum ' + lo + ' is above maximum ' + hi);
    return [lo === null ? -Infinity : lo, hi === null ? Infinity : hi];
  }
  // A number field's or slider's step: above 0, and 1 when absent.
  function step(p) {
    var s = optNumber(p, 'step');
    if (s === null) return 1;
    if (!(s > 0)) throw new Error('step must be above 0, not ' + s);
    return s;
  }
  // The row a drop-down or option group has chosen, checked against its
  // items; null for none.
  function chosen(node, items) {
    var c = values[node.id];
    if (c === null || c === undefined) return null;
    if (!Number.isInteger(c) || c < 0) throw new Error('selected must be a row number, not ' + JSON.stringify(c));
    if (c >= items.length) throw new Error('selected is row ' + c + ' of ' + items.length + ' items');
    return c;
  }
  // Takes an input's new value from a person: stores it, posts Change, and
  // reports the state a page loaded again restores.
  function changed(node, value) {
    values[node.id] = value;
    emit(node, 'Change', value);
    report(node.id);
  }

  function drawNode(node) {
    var p = props(node);
    switch (node.kind) {
      case 'Column':
      case 'Tab':
      case 'Row': {
        // A Column places each widget across by its align; a Row places
        // them as a group, or with stretch shares the room between them.
        var box = el('div', (node.kind === 'Row' ? 'row' : 'col') + ' align-' + pwrsLayout.align(node));
        (node.children || []).forEach(function (c) { box.appendChild(draw(c)); });
        return box;
      }
      case 'Grid': return drawGrid(node, p);
      case 'Group': return drawGroup(node, p);
      case 'Tabs': return drawTabs(node, p);
      case 'Split': return drawSplit(node, p);
      case 'Scroll': return drawScroll(node, p);
      case 'Text': {
        var style = p.style === undefined ? 'body' : p.style;
        if (['heading', 'body', 'mono', 'link', 'instruction'].indexOf(style) < 0) throw new Error('style must be heading, body, mono, link or instruction, not ' + style);
        if (style === 'link') {
          var link = el('button', 'link', need(p, 'text', 'string'));
          link.dataset.node = node.id;
          link.onclick = function () { emit(node, 'Click'); };
          return link;
        }
        // An instruction is a dialog's main line: larger, in the strong
        // text's color.
        var shown = el('div', style === 'body' ? 'label' : 'label ' + style, need(p, 'text', 'string'));
        if (style === 'heading') shown.setAttribute('role', 'heading');
        return shown;
      }
      case 'Image': return drawImage(node, p);
      case 'Markdown': return drawMarkdown(node, p);
      case 'ProgressBar': return drawProgress(node, p);
      case 'Spinner': return spinner();
      case 'LogView': return drawLog(node, p);
      case 'Chart': return drawChart(node, p);
      case 'GpuSurface': return drawSurface(node, p);
      case 'Canvas': return drawCanvas(node, p);
      case 'Button': {
        // The window's default button takes the selection's colors, the
        // accent a selected row is drawn in.
        var b = el('button', 'btn frame' + (pwrsLayout.role(node) === 'default' ? ' default' : ''), need(p, 'text', 'string'));
        b.disabled = p.enabled === false;
        b.dataset.node = node.id;
        b.onclick = function () { emit(node, 'Click'); };
        return b;
      }
      case 'Separator': { var line = el('div', 'separator'); line.setAttribute('role', 'separator'); return line; }
      case 'Spacer': { var s = el('div', 'spacer'); s.style.setProperty('--size', (typeof p.size === 'number' ? p.size : 8) + 'px'); return s; }
      case 'ListBox': return drawList(node, p);
      case 'Table': return drawTable(node, p);
      case 'TreeView': return drawTree(node, p);
      case 'TreeItem': throw new Error("a tree's items are drawn by their TreeView");
      case 'CheckBox': return drawCheck(node, p);
      case 'TextBox': return drawTextBox(node, p);
      case 'NumberBox': return drawNumber(node, p);
      case 'Slider': return drawSlider(node, p);
      case 'ComboBox': return drawCombo(node, p);
      case 'RadioGroup': return drawRadio(node, p);
      case 'ColorPicker': return drawColor(node, p);
      case 'DatePicker': return drawDate(node, p);
      case 'FilePicker': return drawFile(node, p);
      case 'Menu':
      case 'MenuItem': throw new Error("a menu's entries are drawn in its menu bar, menu or context menu");
      default: throw new Error('no widget of kind ' + node.kind);
    }
  }

  // An Image: its picture loaded from the address the helper gives it,
  // at its own size and shrunk to the room across, a width or a height
  // setting that side and scaling the other, and both a box its fit fills
  // as CSS object-fit fills one. A picture that cannot be drawn shows its
  // alt text, or why, and is reported.
  function drawImage(node, p) {
    var picture = need(p, 'image', 'object');
    if (!picture || typeof picture.src !== 'string') throw new Error('image has no address to load the picture from');
    var w = optNumber(p, 'width'), h = optNumber(p, 'height');
    if (w !== null && !(w > 0)) throw new Error('width must be above 0 points, not ' + w);
    if (h !== null && !(h > 0)) throw new Error('height must be above 0 points, not ' + h);
    var fit = p.fit === undefined ? 'contain' : p.fit;
    if (['contain', 'cover', 'fill', 'none'].indexOf(fit) < 0) throw new Error('fit must be contain, cover, fill or none, not ' + fit);
    var img = el('img', 'image');
    img.alt = typeof p.alt === 'string' ? p.alt : '';
    if (w !== null) img.style.width = w + 'px';
    if (h !== null) img.style.height = h + 'px';
    if (w !== null && h !== null) img.style.objectFit = fit;
    if (w === null && h === null) img.style.maxWidth = '100%'; else img.style.flexShrink = '0';
    img.onerror = function () {
      var label = 'Image ' + (node.name || node.id) + ': the picture could not be drawn';
      post({ t: 'error', text: label });
      var shown = el('div', img.alt ? '' : 'error', img.alt || label);
      shown.dataset.node = node.id;
      if (img.parentNode) img.parentNode.replaceChild(shown, img);
    };
    img.src = picture.src;
    return img;
  }

  // The images of each Markdown widget's document whose pictures could not
  // be drawn and have been reported, by the widget, until its document
  // changes.
  var failedPictures = {};

  // A Markdown widget: its document as the HTML the helper wrote, which has
  // only the tags the renderers draw. A link raises Click with its address,
  // or for a heading's {#id} scrolls the document there; an image whose
  // picture cannot be drawn shows its alt text and is reported, once until
  // the document changes. An https picture stands as a spinner the size egui
  // gives a picture still loading until it arrives, and while its window is
  // measured, as egui measures one.
  function drawMarkdown(node, p) {
    var doc = need(p, 'document', 'object');
    if (doc && typeof doc.problem === 'string') throw new Error(doc.problem);
    if (!doc || typeof doc.html !== 'string') throw new Error('document has no HTML to show');
    var failed = failedPictures[node.id];
    if (!failed || failed.doc !== doc) failed = failedPictures[node.id] = { doc: doc, addresses: new Set() };
    var box = el('div', 'markdown');
    box.innerHTML = doc.html;
    // An ordered list's points are its items' numbers from its start, which
    // the stylesheet draws as egui draws them.
    box.querySelectorAll('ol').forEach(function (ol) {
      var number = ol.start;
      Array.prototype.forEach.call(ol.children, function (li) { if (li.tagName === 'LI') li.dataset.number = String(number++); });
    });
    // A table's columns are as egui's grid makes them: each as wide as its
    // widest cell and an interactive widget at least, but the last, which
    // ends at its widest cell.
    box.querySelectorAll('table').forEach(function (table) {
      var head = table.querySelector('tr');
      var columns = head ? head.children.length : 1;
      table.style.gridTemplateColumns = (columns > 1 ? 'repeat(' + (columns - 1) + ', minmax(var(--interact-w), max-content)) ' : '') + 'max-content';
    });
    box.querySelectorAll('a[data-link]').forEach(function (a) {
      a.onclick = function (e) {
        e.preventDefault();
        var to = a.dataset.link;
        if (to.charAt(0) === '#') scrollToHeading(box, to.slice(1)); else emit(node, 'Click', to);
      };
    });
    box.querySelectorAll('img[data-address]').forEach(function (img) {
      var waiting = null;
      if (/^https:/i.test(img.getAttribute('src') || '')) {
        waiting = spinner();
        waiting.classList.add('picture-loading');
        img.classList.add('web');
        img.parentNode.insertBefore(waiting, img.nextSibling);
        if (img.complete && img.naturalWidth > 0) img.classList.add('loaded');
        else img.addEventListener('load', function () { img.classList.add('loaded'); }, { once: true });
      }
      img.onerror = function () {
        var address = img.dataset.address;
        if (!failed.addresses.has(address)) {
          failed.addresses.add(address);
          post({ t: 'error', text: 'Markdown ' + (node.name || node.id) + ': the image ' + address + ' shows its alt text: the picture could not be drawn' });
        }
        if (!img.parentNode) return;
        img.parentNode.replaceChild(waiting ? el('span', 'web-alt', img.alt) : document.createTextNode(img.alt), img);
      };
    });
    return box;
  }

  // Scrolls a document to the heading whose {#id} is `id`.
  function scrollToHeading(box, id) {
    var found = Array.prototype.find.call(box.querySelectorAll('[data-heading]'), function (h) { return h.dataset.heading === id; });
    if (found) found.scrollIntoView({ block: 'start' });
  }

  function drawProgress(node, p) {
    var value = needNumber(p, 'value');
    var maximum = p.maximum === undefined ? 100 : needNumber(p, 'maximum');
    if (!(maximum > 0)) throw new Error('maximum must be above 0, not ' + maximum);
    if (value < 0 || value > maximum) throw new Error('value must be from 0 to ' + maximum + ', not ' + value);
    if (p.text !== undefined && typeof p.text !== 'string') throw new Error('text must be a string');
    var box = el('div', 'progress');
    box.setAttribute('role', 'progressbar');
    box.setAttribute('aria-valuemin', '0');
    box.setAttribute('aria-valuemax', String(maximum));
    box.setAttribute('aria-valuenow', String(value));
    if (p.text) box.setAttribute('aria-label', p.text);
    var line = el('div', 'line');
    line.appendChild(el('span', 'said', p.text !== undefined ? p.text : ''));
    line.appendChild(el('span', 'done', Math.floor(value / maximum * 100) + '%'));
    box.appendChild(line);
    var rail = el('div', 'rail');
    var bar = el('div', 'bar');
    bar.style.width = (value / maximum * 100) + '%';
    rail.appendChild(bar);
    box.appendChild(rail);
    return box;
  }

  // A log's lines in a box that scrolls, as wide and as high as its width
  // and height where they are given, however many lines it has.
  function drawLog(node, p) {
    var lines = needStrings(p, 'lines');
    var box = el('div', 'log mono');
    ['width', 'height'].forEach(function (key) {
      if (p[key] === undefined || p[key] === null) return;
      if (typeof p[key] !== 'number' || !(p[key] >= 0)) throw new Error(key + ' must be 0 or more points');
      box.style[key] = p[key] + 'px';
    });
    lines.forEach(function (l) { box.appendChild(el('div', 'line', l)); });
    box.onscroll = function () { logEnds[node.id] = box.scrollTop + box.clientHeight >= box.scrollHeight - 2; };
    return box;
  }
  // Adds a log's new lines to it where it is drawn, scrolling it to its
  // newest line unless a person has scrolled up; a log drawn as an error,
  // or given a line that is not text, is drawn again whole.
  function addLines(node, lines) {
    var box = root.querySelector('[data-node="' + node.id + '"]');
    if (!box) return;
    if (!box.classList.contains('log') || lines.some(function (l) { return typeof l !== 'string'; })) { redraw(node); return; }
    lines.forEach(function (l) { box.appendChild(el('div', 'line', l)); });
    if (logEnds[node.id] !== false) box.scrollTop = box.scrollHeight;
  }

  // A grid: its widgets in rows of the given number of columns, each column
  // as wide as its widest widget and at least the interact width, as egui's
  // grid makes it, each widget placed in its cell by the grid's align; with
  // stretch the last column also takes the room left across, its widgets
  // marked as markColumns marks them. In a grid that fills down, the rows
  // with a widget that fills share the height the others leave, three
  // rows of controls each at least.
  function drawGrid(node, p) {
    if (!Number.isInteger(p.columns) || p.columns < 1) throw new Error('columns must be a whole number above 0');
    var align = pwrsLayout.align(node);
    var box = el('div', 'grid align-' + align);
    box.dataset.columns = String(p.columns);
    var column = 'minmax(var(--interact-w), max-content)';
    box.style.gridTemplateColumns = align === 'stretch'
      ? (p.columns > 1 ? 'repeat(' + (p.columns - 1) + ', ' + column + ') ' : '') + 'minmax(var(--interact-w), 1fr)'
      : 'repeat(' + p.columns + ', ' + column + ')';
    if (pwrsLayout.fillsDown(node)) {
      var filling = [];
      (node.children || []).filter(pwrsLayout.inFlow).forEach(function (c, i) {
        var row = Math.floor(i / p.columns);
        filling[row] = filling[row] === true || pwrsLayout.fillsDown(c);
      });
      box.style.gridTemplateRows = filling.map(function (fills) { return fills ? 'minmax(calc(3 * var(--interact-h)), 1fr)' : 'minmax(var(--interact-h), auto)'; }).join(' ');
    }
    (node.children || []).forEach(function (c) { box.appendChild(draw(c)); });
    markColumns(box);
    return box;
  }
  // Marks each widget of a grid's last column `last`, counting the cells as
  // egui's grid counts them, so a stretched grid's rules size the fields of
  // that column apart from the others.
  function markColumns(grid) {
    var columns = parseInt(grid.dataset.columns, 10);
    Array.prototype.forEach.call(grid.children, function (c, i) { c.classList.toggle('last', (i + 1) % columns === 0); });
  }

  // A group: egui's group frame round its widgets, its title first among
  // them in the strong text's color.
  function drawGroup(node, p) {
    if (p.title !== undefined && typeof p.title !== 'string') throw new Error('title must be a string');
    var box = el('div', 'group');
    box.setAttribute('role', 'group');
    var inner = el('div', 'col');
    if (p.title !== undefined) inner.appendChild(el('div', 'label strong', p.title));
    (node.children || []).forEach(function (c) { inner.appendChild(draw(c)); });
    box.appendChild(inner);
    return box;
  }

  // A set of tabs: each title a tab's button in a row, a separator under
  // them, and the chosen tab's widgets.
  function drawTabs(node) {
    var tabs = node.children || [];
    var titles = tabs.map(function (t) {
      if (t.kind !== 'Tab') throw new Error('a Tabs contains only Tab widgets, and this one contains a ' + t.kind);
      return need(props(t), 'title', 'string');
    });
    var c = chosen(node, titles);
    var box = el('div', 'tabs');
    var bar = el('div', 'tabbar');
    bar.setAttribute('role', 'tablist');
    titles.forEach(function (title, i) {
      var b = el('button', 'btn tab' + (c === i ? ' picked' : ''), title);
      b.setAttribute('role', 'tab');
      b.setAttribute('aria-selected', c === i ? 'true' : 'false');
      b.dataset.node = node.id + '-' + i;
      b.onclick = function () { if (values[node.id] !== i) { changed(node, i); redraw(node); } };
      bar.appendChild(b);
    });
    box.appendChild(bar);
    var line = el('div', 'separator');
    line.setAttribute('role', 'separator');
    box.appendChild(line);
    if (c !== null) box.appendChild(draw(tabs[c]));
    return box;
  }

  function drawSplit(node, p) {
    var panes = node.children || [];
    if (panes.length !== 2) throw new Error('a Split contains two widgets, and this one contains ' + panes.length);
    var orientation = p.orientation === undefined ? 'horizontal' : p.orientation;
    if (orientation !== 'horizontal' && orientation !== 'vertical') throw new Error('orientation must be horizontal or vertical, not ' + orientation);
    var vertical = orientation === 'vertical';
    if (vertical && inScroll(node.id)) throw new Error('a vertical Split takes the height the window leaves it, and inside a Scroll that height has no end');
    var share = splits[node.id];
    if (typeof share !== 'number' || !(share >= 0 && share <= 1)) throw new Error('position must be from 0 to 1');
    var box = el('div', 'split ' + orientation);
    var first = el('div', 'pane'), second = el('div', 'pane'), divider = el('div', 'divider');
    var place = function (s) { box.style.setProperty('--share', String(s)); };
    place(share);
    first.appendChild(draw(panes[0]));
    second.appendChild(draw(panes[1]));
    divider.onpointerdown = function (down) {
      divider.setPointerCapture(down.pointerId);
      divider.onpointermove = function (e) {
        var r = box.getBoundingClientRect();
        var size = vertical ? divider.offsetHeight : divider.offsetWidth;
        var extent = (vertical ? r.height : r.width) - size;
        if (extent <= 0) return;
        var at = (vertical ? e.clientY - r.top : e.clientX - r.left) - size / 2;
        splits[node.id] = Math.min(Math.max(at / extent, 0), 1);
        place(splits[node.id]);
      };
      divider.onpointerup = function () { divider.onpointermove = null; report(node.id); };
    };
    box.appendChild(first);
    box.appendChild(divider);
    box.appendChild(second);
    return box;
  }

  function drawScroll(node, p) {
    var box = el('div', 'scroll-area');
    if (p.height !== undefined && p.height !== null) {
      if (typeof p.height !== 'number' || !(p.height >= 0)) throw new Error('height must be 0 or more points');
      box.style.maxHeight = p.height + 'px';
    }
    var inner = el('div', 'col');
    (node.children || []).forEach(function (c) { inner.appendChild(draw(c)); });
    box.appendChild(inner);
    return box;
  }

  // The row key a press stands for in a list, table or tree, with Shift
  // and Ctrl as pressed (pwrsLayout.rowKeyNamed); null for a press the
  // widgets leave to the window, and for a repeat of Space, Enter or
  // Ctrl+A, which act once, where a move repeats while its key is down, as
  // in egui.
  function rowKeyOf(e) {
    var k = pressedKeys(e);
    var key = k ? pwrsLayout.rowKeyNamed(k.key, k.ctrl, k.shift, k.alt) : null;
    if (key === null || (e.repeat && ['Space', 'Enter', 'All'].indexOf(key) >= 0)) return null;
    return { key: key, shift: k.shift, ctrl: k.ctrl };
  }
  // How many rows a page key moves by in `scroller`, whose rows are as tall
  // as `row`, the first of them (pwrsLayout.pageRows).
  function pageOf(scroller, row) {
    return pwrsLayout.pageRows(scroller.clientHeight, row ? row.getBoundingClientRect().height : 0);
  }
  // Scrolls the row the keyboard is on, of the list, table or tree drawn
  // for `node`, into sight.
  function showFocused(node) {
    var drawn = root.querySelector('[data-node="' + node.id + '"]');
    var row = drawn && drawn.querySelector('.focused');
    if (row) row.scrollIntoView({ block: 'nearest' });
  }

  // A list: its items as selectable rows, or with multiple as check boxes,
  // in a box that takes the focus as one widget, a click on a row giving
  // it to the box. With the focus, the keys pwrsLayout.rowKey gives move
  // and select, check boxes moving without checking, and the keyboard's
  // row shows a ring and scrolls into sight.
  function drawList(node, p) {
    if (!Array.isArray(p.items) || p.items.some(function (i) { return typeof i !== 'string'; })) throw new Error('items must be a list of strings');
    var state = lists[node.id] || (lists[node.id] = { selected: new Set() });
    if (state.current === undefined) state.current = null;
    if (state.anchor === undefined) state.anchor = null;
    var multiple = p.multiple === true;
    var shown = p.items.map(function (text, i) { return i; });
    var box = el('div', 'list');
    box.tabIndex = 0;
    box.dataset.node = node.id;
    box.setAttribute('role', 'listbox');
    if (multiple) box.setAttribute('aria-multiselectable', 'true');
    var toBox = function (e) { e.preventDefault(); box.focus({ preventScroll: true }); };
    var clicked = function (i, select) {
      state.current = i;
      state.anchor = i;
      if (pwrsLayout.applyRows(select, state.selected, shown)) {
        emit(node, 'Select', sorted(state.selected));
        report(node.id);
      }
      redraw(node);
    };
    p.items.forEach(function (text, i) {
      var current = state.current === i ? ' focused' : '';
      if (multiple) {
        var made = makeCheck(text, state.selected.has(i), true, function () { clicked(i, { kind: 'toggle', at: i }); });
        made.box.tabIndex = -1;
        made.label.onmousedown = toBox;
        if (current) made.label.classList.add('focused');
        made.label.setAttribute('role', 'option');
        made.label.setAttribute('aria-selected', state.selected.has(i) ? 'true' : 'false');
        box.appendChild(made.label);
        return;
      }
      var item = el('button', 'btn frame selectable' + (state.selected.has(i) ? ' picked' : '') + current, text);
      item.tabIndex = -1;
      item.onmousedown = toBox;
      item.setAttribute('role', 'option');
      item.setAttribute('aria-selected', state.selected.has(i) ? 'true' : 'false');
      item.onclick = function () { clicked(i, { kind: 'only', at: i }); };
      item.ondblclick = function () { emit(node, 'Activate', i); };
      box.appendChild(item);
    });
    // Not at a press, whose click puts the keyboard on the item clicked.
    box.onfocus = function () {
      if ((state.current !== null && state.current < p.items.length) || pointerNow.down.some(Boolean)) return;
      state.current = state.selected.size ? sorted(state.selected)[0] : p.items.length ? 0 : null;
      redraw(node);
    };
    box.onkeydown = function (e) {
      var k = rowKeyOf(e);
      if (!k) return;
      e.preventDefault();
      var m = pwrsLayout.rowKey(k.key, k.shift, k.ctrl, { count: p.items.length, selection: multiple ? 'checks' : 'single', current: state.current, currentSelected: state.current !== null && state.selected.has(state.current), anchor: state.anchor, page: pageOf(box, box.firstElementChild) });
      if (!m) return;
      state.current = m.current;
      state.anchor = m.anchor;
      if (pwrsLayout.applyRows(m.select, state.selected, shown)) {
        emit(node, 'Select', sorted(state.selected));
        report(node.id);
      }
      if (m.activate) emit(node, 'Activate', m.current);
      redraw(node);
      showFocused(node);
    };
    return box;
  }

  // The arrows a table's header gives the column it is sorted by, up and then
  // down, as egui's table gives them.
  var SORT_MARKS = [' ↑', ' ↓'];

  // A table: its filter row when it has one, then a header of headings in
  // the strong text color, each a button framed only under the pointer that
  // sorts by its column, with a line under it, over its rows; each column's
  // heading and cells placed across as its align says, with a line between
  // each two columns (columnEdges). Each column has the room its heading
  // takes with the wider sort arrow, so sorting moves no column, or takes
  // the width a person dragged it to. Its rows take the focus as one
  // part, the filter field a Tab stop of its own; with the focus, the keys
  // pwrsLayout.rowKey gives move and select, and the keyboard's row shows a
  // ring and scrolls into sight. Rows Add-GuiItem adds while it shows its
  // last row leave the last row in sight, as fit says.
  function drawTable(node, p) {
    var mode = p.selection === undefined ? 'none' : p.selection;
    if (['none', 'single', 'multiple'].indexOf(mode) < 0) throw new Error('selection must be none, single or multiple, not ' + mode);
    var t = tables[node.id];
    refresh(t);
    if (t.current === undefined) t.current = null;
    if (t.anchor === undefined) t.anchor = null;
    var box = el('div', 'table');
    if (flag(p, 'filter', false)) {
      var tools = el('div', 'tools');
      tools.appendChild(el('span', 'label', 'Filter'));
      var filter = el('input', 'field');
      filter.type = 'text';
      filter.value = t.filter;
      filter.dataset.node = node.id + '-filter';
      filter.oninput = function () { t.filter = filter.value; t.dirty = true; report(node.id); redraw(node); };
      tools.appendChild(filter);
      tools.appendChild(el('span', 'label', t.view.length + ' of ' + t.rows.length));
      box.appendChild(tools);
    } else if (t.filter) {
      // A table without its filter row shows every row.
      t.filter = '';
      t.dirty = true;
      refresh(t);
    }
    var scroll = el('div', 'scroll');
    scroll.tabIndex = 0;
    scroll.dataset.node = node.id + '-rows';
    scroll.onscroll = function () { t.end = scroll.scrollTop + scroll.clientHeight >= scroll.scrollHeight - 2; };
    var table = el('table');
    var head = el('tr');
    // Where each column's header and cells sit across: as its align says,
    // else at the right when its first row's cell is an amount, as
    // PowerShell's tables place amounts, and at the left, a date among them.
    var aligns = t.columns.map(function (column, i) {
      if (column.align !== null) {
        if (['left', 'center', 'right'].indexOf(column.align) < 0) throw new Error("a column's align must be left, center or right, not " + column.align);
        return column.align;
      }
      var first = t.rows.length ? t.rows[0][i] : null;
      return first && typeof first.sort === 'number' && first.date !== true ? 'right' : 'left';
    });
    // The last column takes the room the table has to spare only when its
    // cells stand at the left, as text does; a column of numbers or of
    // centered values stays at its own width, ended as a last column is, and an
    // empty column after it takes the room, so its values stay beside the
    // rest of their row.
    var filler = aligns.length > 0 && aligns[aligns.length - 1] !== 'left';
    var end = function (i) { return filler && i === aligns.length - 1 ? 'end' : ''; };
    t.columns.forEach(function (column, i) {
      var arrow = t.sort && t.sort[0] === i ? SORT_MARKS[t.sort[1] ? 0 : 1] : '';
      var th = el('th', end(i));
      th.style.textAlign = aligns[i];
      if (typeof t.widths[i] === 'number') th.style.width = t.widths[i] + 'px';
      var sort = el('button', 'btn frame selectable heading', column.label + arrow);
      sort.onclick = function () { t.sort = t.sort && t.sort[0] === i && t.sort[1] ? [i, false] : [i, true]; t.dirty = true; report(node.id); redraw(node); };
      th.appendChild(sort);
      SORT_MARKS.forEach(function (mark) {
        var room = el('span', 'btn room', column.label + mark);
        room.setAttribute('aria-hidden', 'true');
        th.appendChild(room);
      });
      head.appendChild(th);
    });
    if (filler) head.appendChild(el('th', 'filler'));
    var thead = el('thead');
    thead.appendChild(head);
    table.appendChild(thead);
    var tbody = el('tbody');
    table.appendChild(tbody);
    t.view.forEach(function (r, at) {
      var tr = el('tr', (t.selected.has(r) ? 'picked' : '') + (t.current === r ? ' focused' : ''));
      t.rows[r].forEach(function (c, i) {
        var td = el('td', end(i), c.text);
        td.style.textAlign = aligns[i] || 'left';
        tr.appendChild(td);
      });
      if (filler) tr.appendChild(el('td', 'filler'));
      // A click selects its row, with multiple Ctrl adding or taking it
      // away and Shift taking the range from the anchor, which stays; it
      // raises Select even where the selection stays the same.
      tr.onclick = function (e) {
        var ranged = mode === 'multiple' && e.shiftKey && t.anchor !== null;
        if (mode !== 'none') {
          var from = t.view.indexOf(t.anchor);
          var select = mode === 'multiple' && (e.ctrlKey || e.metaKey) ? { kind: 'toggle', at: at } : ranged ? { kind: 'range', at: from < 0 ? at : from, to: at } : { kind: 'only', at: at };
          pwrsLayout.applyRows(select, t.selected, t.view);
          emit(node, 'Select', sorted(t.selected));
        }
        if (!ranged) t.anchor = r;
        t.current = r;
        report(node.id);
        redraw(node);
      };
      tr.ondblclick = function () { if (mode !== 'none') emit(node, 'Activate', r); };
      tbody.appendChild(tr);
    });
    // Rows are named by their places in the rows as shown, sorted and
    // filtered, for the rule; null for one not shown.
    var place = function (r) { var i = r === null ? -1 : t.view.indexOf(r); return i < 0 ? null : i; };
    // The rows taking the focus put the keyboard on the first row when it is
    // on none, but not at a press, whose click puts it on the row clicked
    // and would find its row drawn again before its release.
    scroll.onfocus = function () {
      if (place(t.current) !== null || pointerNow.down.some(Boolean)) return;
      t.current = t.view.length ? t.view[0] : null;
      redraw(node);
    };
    scroll.onkeydown = function (e) {
      var k = rowKeyOf(e);
      if (!k) return;
      e.preventDefault();
      var m = pwrsLayout.rowKey(k.key, k.shift, k.ctrl, { count: t.view.length, selection: mode, current: place(t.current), currentSelected: t.current !== null && t.selected.has(t.current), anchor: place(t.anchor), page: pageOf(scroll, tbody.firstElementChild) });
      if (!m) return;
      t.current = t.view[m.current];
      t.anchor = m.anchor === null ? null : t.view[m.anchor];
      if (pwrsLayout.applyRows(m.select, t.selected, t.view)) emit(node, 'Select', sorted(t.selected));
      if (m.activate) emit(node, 'Activate', t.current);
      report(node.id);
      redraw(node);
      showFocused(node);
    };
    scroll.appendChild(table);
    box.appendChild(scroll);
    box.appendChild(columnEdges(t, box, scroll, head, filler));
    return box;
  }

  // The lines between table `t`'s columns, as egui_extras draws them: one in
  // the middle of the gap after each column but the last, the empty column
  // that takes the spare room counting as the last when the table has one,
  // from the top of the header to the bottom of the rows' room, placed again
  // whenever a column, the room or the scroll moves. A drag on one makes the
  // column before it as wide as the pointer takes it, no narrower than its
  // content and its header's room, and a double click returns the column to
  // them.
  function columnEdges(t, box, scroll, head, filler) {
    var layer = el('div', 'edges');
    var lines = [];
    var place = function () {
      var at = box.getBoundingClientRect(), view = scroll.getBoundingClientRect();
      lines.forEach(function (line, i) {
        line.style.left = (head.children[i].getBoundingClientRect().right - at.left) + 'px';
        line.style.top = (view.top - at.top) + 'px';
        line.style.height = scroll.clientHeight + 'px';
      });
    };
    t.columns.slice(0, filler ? t.columns.length : -1).forEach(function (column, i) {
      var line = el('div', 'edge');
      line.onpointerdown = function (down) {
        if (down.button !== 0) return;
        down.preventDefault();
        line.setPointerCapture(down.pointerId);
        line.classList.add('dragged');
        var th = head.children[i], from = th.getBoundingClientRect().width;
        line.onpointermove = function (e) {
          t.widths[i] = from + e.clientX - down.clientX;
          th.style.width = t.widths[i] + 'px';
          place();
        };
        line.onpointerup = line.onpointercancel = function () { line.onpointermove = null; line.classList.remove('dragged'); };
      };
      line.ondblclick = function () { t.widths[i] = undefined; head.children[i].style.width = ''; place(); };
      lines.push(line);
      layer.appendChild(line);
    });
    if (t.edges) t.edges.disconnect();
    t.edges = new ResizeObserver(place);
    t.edges.observe(scroll);
    Array.prototype.forEach.call(head.children, function (th) { t.edges.observe(th); });
    scroll.addEventListener('scroll', place);
    return layer;
  }

  // Fails unless every node under a tree is a tree item with its text.
  function checkItems(items) {
    (items || []).forEach(function (it) {
      if (it.kind !== 'TreeItem') throw new Error('a TreeView contains only TreeItem nodes, and node ' + it.id + ' is a ' + it.kind);
      need(props(it), 'text', 'string');
      flag(props(it), 'more', false);
      checkItems(it.children);
    });
  }
  // A tree's items as shown, top to bottom, each with its depth: an item,
  // then, while it is open, the items inside it.
  function shownItems(items, depth, out) {
    (items || []).forEach(function (it) {
      out.push({ item: it, depth: depth });
      if (openItems.has(it.id)) shownItems(it.children, depth + 1, out);
    });
    return out;
  }
  // Every item of a tree in tree order, open or not.
  function treeOrder(items, out) {
    (items || []).forEach(function (it) { out.push(it.id); treeOrder(it.children, out); });
    return out;
  }
  // A tree's selection as its Select event gives it: the items' node ids
  // in tree order.
  function selectionOf(tv, node) { return treeOrder(node.children, []).filter(function (id) { return tv.selected.has(id); }); }
  function opens(it) { return (it.children || []).length > 0 || props(it).more === true; }
  function lazy(it) { return !(it.children || []).length && props(it).more === true; }
  // Opens or closes a tree item as a person does. Opening one whose
  // children are still to come raises Expand on the tree, once, and the
  // item waits for them.
  function toggleItem(it, node) {
    if (openItems.has(it.id)) { openItems.delete(it.id); return; }
    openItems.add(it.id);
    if (lazy(it) && !expanding.has(it.id)) { expanding.add(it.id); emit(node, 'Expand', it.id); }
  }
  // Selects the item on row `at` of the rows shown, `ids`, as a click with
  // the keys pressed does: alone; in a tree of several, Ctrl adds it or
  // takes it away, and Shift selects the rows from the anchor's to it, as
  // shown. Returns whether the selection changed.
  function selectRow(tv, ids, at, multiple, held) {
    var id = ids[at];
    var before = sorted(tv.selected).join(',');
    if (multiple && held.command) {
      if (!tv.selected.delete(id)) tv.selected.add(id);
      tv.anchor = id;
    } else if (multiple && held.shift) {
      var from = tv.anchor === null ? -1 : ids.indexOf(tv.anchor);
      if (from < 0) from = at;
      tv.selected = new Set(ids.slice(Math.min(from, at), Math.max(from, at) + 1));
    } else {
      tv.selected = new Set([id]);
      tv.anchor = id;
    }
    tv.focus = id;
    return sorted(tv.selected).join(',') !== before;
  }
  // The tree whose items include node `id`; null for none.
  function holderOf(id) {
    var found = null;
    (function walk(n) {
      if (found) return;
      if (n.kind === 'TreeView') { if (n.id !== id && find(n, id)) found = n; return; }
      (n.children || []).forEach(walk);
    })(tree);
    return found;
  }

  // A tree: a row per item shown, indented by its depth, an arrow before
  // each item that opens, its text ending in an ellipsis where the tree is
  // too narrow for it, and a spinner after one waiting for its children.
  // A click selects as selectRow says, the arrow opens or closes,
  // a double click activates. With the tree focused, Up, Down, Home and End
  // move to a row and select as a click there would, Ctrl moving without
  // selecting in a tree of several; Right opens an item or moves into an
  // open one; Left closes an open item or moves to its parent; Space
  // selects the row, or adds or takes it away in a tree of several; Enter
  // activates it.
  function drawTree(node, p) {
    var multiple = flag(p, 'multiple', false);
    checkItems(node.children);
    var tv = trees[node.id] || (trees[node.id] = { selected: new Set(), anchor: null, focus: null });
    var rows = shownItems(node.children, 0, []);
    var ids = rows.map(function (r) { return r.item.id; });
    var box = el('div', 'tree');
    box.tabIndex = 0;
    box.dataset.node = node.id;
    box.setAttribute('role', 'tree');
    if (multiple) box.setAttribute('aria-multiselectable', 'true');
    var after = function (changed, activated) {
      if (changed) emit(node, 'Select', selectionOf(tv, node));
      if (activated !== null) emit(node, 'Activate', activated);
      report(node.id);
      redraw(node);
    };
    rows.forEach(function (r, at) {
      var it = r.item;
      var row = el('div', 'treeitem' + (tv.selected.has(it.id) ? ' picked' : '') + (tv.focus === it.id ? ' focused' : ''));
      row.setAttribute('role', 'treeitem');
      row.setAttribute('aria-level', String(r.depth + 1));
      row.setAttribute('aria-selected', tv.selected.has(it.id) ? 'true' : 'false');
      row.style.paddingLeft = 'calc(var(--indent) * ' + r.depth + ')';
      var arrow = el('span', 'arrow');
      if (opens(it)) {
        arrow.classList.add(openItems.has(it.id) ? 'open' : 'closed');
        row.setAttribute('aria-expanded', openItems.has(it.id) ? 'true' : 'false');
        arrow.onclick = function (e) { e.stopPropagation(); toggleItem(it, node); tv.focus = it.id; after(false, null); };
      }
      row.appendChild(arrow);
      row.appendChild(el('span', 'name', props(it).text));
      if (expanding.has(it.id)) row.appendChild(spinner());
      row.onclick = function (e) { after(selectRow(tv, ids, at, multiple, { command: e.ctrlKey || e.metaKey, shift: e.shiftKey }), null); };
      row.ondblclick = function () { after(false, it.id); };
      box.appendChild(row);
    });
    // The keys pwrsLayout.rowKey gives move and select; Right opens an item
    // or moves into an open one, and Left closes an open item or moves to
    // its parent. The keyboard's row scrolls into sight.
    var place = function (id) { var i = id === null ? -1 : ids.indexOf(id); return i < 0 ? null : i; };
    // Not at a press, whose click puts the keyboard on the row clicked.
    box.onfocus = function () {
      if (place(tv.focus) !== null || !ids.length || pointerNow.down.some(Boolean)) return;
      tv.focus = ids[0];
      redraw(node);
    };
    box.onkeydown = function (e) {
      if (!rows.length) return;
      var k = rowKeyOf(e);
      if (k) {
        e.preventDefault();
        var m = pwrsLayout.rowKey(k.key, k.shift, k.ctrl, { count: ids.length, selection: multiple ? 'multiple' : 'single', current: place(tv.focus), currentSelected: tv.focus !== null && tv.selected.has(tv.focus), anchor: place(tv.anchor), page: pageOf(box, box.firstElementChild) });
        if (!m) return;
        tv.focus = ids[m.current];
        tv.anchor = m.anchor === null ? null : ids[m.anchor];
        after(pwrsLayout.applyRows(m.select, tv.selected, ids), m.activate ? tv.focus : null);
        showFocused(node);
        return;
      }
      var at = place(tv.focus);
      if (at === null || e.altKey || (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft')) return;
      var last = rows.length - 1, item = rows[at].item, changed = false;
      if (e.key === 'ArrowRight') {
        if (opens(item) && !openItems.has(item.id)) toggleItem(item, node);
        else if (at < last && rows[at + 1].depth > rows[at].depth) changed = selectRow(tv, ids, at + 1, multiple, {});
      } else if (opens(item) && openItems.has(item.id)) {
        toggleItem(item, node);
      } else {
        for (var up = at - 1; up >= 0; up--) {
          if (rows[at].depth > 0 && rows[up].depth === rows[at].depth - 1) { changed = selectRow(tv, ids, up, multiple, {}); break; }
        }
      }
      e.preventDefault();
      after(changed, null);
      showFocused(node);
    };
    return box;
  }

  // A check box as egui draws one: its box, the check mark egui's line
  // draws in it while checked, and its text; `onChange` takes a person's
  // change.
  function makeCheck(text, checked, enabled, onChange) {
    var label = el('label', 'check' + (enabled ? '' : ' off'));
    var box = el('input');
    box.type = 'checkbox';
    box.checked = checked;
    box.disabled = !enabled;
    box.onchange = function () { onChange(box.checked); };
    var mark = svgEl('svg', { 'class': 'mark', viewBox: '0 0 8 8' });
    mark.appendChild(svgEl('polyline', { points: '0,4 4,8 8,0' }));
    label.appendChild(box);
    label.appendChild(mark);
    label.appendChild(el('span', 'label', text));
    return { label: label, box: box };
  }
  function drawCheck(node, p) {
    var text = need(p, 'text', 'string');
    flag(p, 'checked', false);
    var made = makeCheck(text, values[node.id] === true, flag(p, 'enabled', true), function (on) { changed(node, on); });
    made.box.dataset.node = node.id + '-input';
    return made.label;
  }

  function drawTextBox(node, p) {
    need(p, 'text', 'string');
    var multiline = flag(p, 'multiline', false), password = flag(p, 'password', false);
    if (password && multiline) throw new Error('a password field is one line');
    if (p.placeholder !== undefined && typeof p.placeholder !== 'string') throw new Error('placeholder must be a string');
    var field = el(multiline ? 'textarea' : 'input', 'field');
    if (!multiline) field.type = password ? 'password' : 'text';
    field.value = values[node.id];
    if (p.placeholder !== undefined) field.placeholder = p.placeholder;
    field.disabled = !flag(p, 'enabled', true);
    field.spellcheck = false;
    field.dataset.node = node.id;
    field.oninput = function () { changed(node, field.value); if (multiline) grow(field); };
    if (!multiline) field.onkeydown = function (e) { if (e.key === 'Enter') { emit(node, 'Submit', field.value); e.preventDefault(); } };
    return password ? peekField(node, field) : field;
  }

  // A password field in a box with its peek button at its right end,
  // offered as pwrsLayout.peek says: the password shows while the button
  // is held, while Alt+F8 is held, or while a test holds it. A press on the
  // button keeps the focus in the field.
  function peekField(node, field) {
    var box = el('span', 'peek-field');
    box.dataset.node = node.id;
    field.dataset.node = node.id + '-input';
    var eye = el('span', 'peek');
    eye.setAttribute('aria-hidden', 'true');
    var rule = peeks[node.id] || (peeks[node.id] = pwrsLayout.peek(field.value));
    var held = false, keyed = false;
    var show = function () {
      var on = rule.offered() && (held || keyed || peekHeld[node.id] === true);
      box.classList.toggle('offered', rule.offered());
      box.classList.toggle('shown', on);
      if ((field.type === 'text') === on) return;
      var from = field.selectionStart, to = field.selectionEnd;
      field.type = on ? 'text' : 'password';
      if (document.activeElement === field) field.setSelectionRange(from, to);
    };
    var sync = function () {
      if (!rule.update(focusedNode() === node.id, field.value)) {
        delete peekHeld[node.id];
        held = keyed = false;
      }
      show();
    };
    eye.onmousedown = function (e) { e.preventDefault(); };
    eye.onpointerdown = function (e) {
      if (e.button !== 0 || !rule.offered()) return;
      eye.setPointerCapture(e.pointerId);
      held = true;
      show();
    };
    eye.onpointerup = eye.onpointercancel = function () { held = false; show(); };
    field.addEventListener('input', sync);
    field.addEventListener('focus', sync);
    // A field drawn again loses the focus as it is replaced and is given it
    // back at once, so the focus has left only when it is elsewhere once
    // the script running now ends.
    field.addEventListener('blur', function () { queueMicrotask(sync); });
    field.addEventListener('keydown', function (e) {
      var keys = pressedKeys(e);
      if (keys && isPeek(keys) && rule.offered()) { keyed = true; show(); e.preventDefault(); }
    });
    field.addEventListener('keyup', function (e) { if (keyed && (e.key === 'F8' || e.key === 'Alt')) { keyed = false; show(); } });
    peekShows[node.id] = show;
    peekSyncs[node.id] = sync;
    box.appendChild(field);
    box.appendChild(eye);
    show();
    return box;
  }
  // Gives every password field's peek rule its field's focus and text now.
  function syncPeeks() {
    Object.keys(peekSyncs).forEach(function (id) { if (find(tree, Number(id))) peekSyncs[id](); });
  }

  var colorForm = /^#[0-9a-f]{6}$/;
  // A color chooser: a swatch of its color, a control's size, which opens
  // egui's color picker under it.
  function drawColor(node, p) {
    need(p, 'color', 'string');
    var hex = values[node.id];
    if (!colorForm.test(hex)) throw new Error('color must be #rrggbb, not ' + hex);
    var swatch = el('button', 'swatch');
    swatch.style.setProperty('--chosen', hex);
    swatch.disabled = !flag(p, 'enabled', true);
    swatch.dataset.node = node.id;
    swatch.setAttribute('aria-label', hex);
    swatch.setAttribute('aria-haspopup', 'dialog');
    swatch.setAttribute('aria-expanded', 'false');
    swatch.onclick = function () {
      if (popup && popup.anchor === swatch) { closePopup(); return; }
      openPicker(swatch, node);
    };
    return swatch;
  }

  // ecolor's conversions, which egui's picker reads and draws colors by: a
  // byte is gamma-encoded, hue, saturation and value work on linear light,
  // and the picker shows and drags the value in gamma.
  function linearFromByte(b) { return b <= 10 ? b / 3294.6 : Math.pow((b + 14.025) / 269.025, 2.4); }
  function byteFromLinear(l) { return l <= 0 ? 0 : l <= 0.0031308 ? Math.floor(3294.6 * l + 0.5) : l <= 1 ? Math.floor(269.025 * Math.pow(l, 1 / 2.4) - 14.025 + 0.5) : 255; }
  function gammaFromLinear(l) { return l <= 0 ? 0 : l <= 0.0031308 ? 12.92 * l : 1.055 * Math.pow(l, 1 / 2.4) - 0.055; }
  function linearFromGamma(g) { return g <= 0.04045 ? g / 12.92 : Math.pow((g + 0.055) / 1.055, 2.4); }
  function rgbFromHsv(h, s, v) {
    h = ((h % 1) + 1) % 1;
    s = Math.min(Math.max(s, 0), 1);
    var i = Math.floor(h * 6), f = h * 6 - i, p = v * (1 - s), q = v * (1 - f * s), t = v * (1 - (1 - f) * s);
    switch (i % 6) {
      case 0: return [v, t, p];
      case 1: return [q, v, p];
      case 2: return [p, v, t];
      case 3: return [p, q, v];
      case 4: return [t, p, v];
      default: return [v, p, q];
    }
  }
  function hsvFromRgb(r, g, b) {
    var lo = Math.min(r, g, b), hi = Math.max(r, g, b), range = hi - lo;
    var h = hi === lo ? 0 : hi === r ? (g - b) / (6 * range) : hi === g ? (b - r) / (6 * range) + 1 / 3 : (r - g) / (6 * range) + 2 / 3;
    return [(h + 1) % 1, hi === 0 ? 0 : 1 - lo / hi, hi];
  }
  // A picker's color { h, s, v }, its value in gamma: as linear channels,
  // as bytes, and back.
  function linearOf(c) { return rgbFromHsv(c.h, c.s, linearFromGamma(c.v)); }
  function bytesOf(c) { return linearOf(c).map(byteFromLinear); }
  function colorOfLinear(l) { var hsv = hsvFromRgb(l[0], l[1], l[2]); return { h: hsv[0], s: hsv[1], v: gammaFromLinear(hsv[2]) }; }
  function colorOfBytes(b) { return colorOfLinear(b.map(linearFromByte)); }
  function bytesOfHex(hex) { return [1, 3, 5].map(function (i) { return parseInt(hex.slice(i, i + 2), 16); }); }
  function hexOf(b) { return '#' + b.map(function (n) { return ('0' + n.toString(16)).slice(-2); }).join(''); }
  // White or black, whichever shows on a color, as egui marks a picked one.
  function contrastOn(b) { var l = b.map(linearFromByte); return 0.3 * l[0] + 0.59 * l[1] + 0.11 * l[2] < 0.5 ? '#ffffff' : '#000000'; }

  // How wide egui's picker draws its color, square and hue bar, in points.
  var pickerWidth = 275;
  // Paints a picker's square of saturation across and value up at the
  // color's hue, and the circle egui marks the color with.
  function paintSquare(canvas, c) {
    var ratio = window.devicePixelRatio || 1, w = Math.round(pickerWidth * ratio), h = w;
    canvas.width = w;
    canvas.height = h;
    var g = canvas.getContext('2d'), image = g.createImageData(w, h), d = image.data;
    for (var y = 0; y < h; y++) {
      var v = linearFromGamma(1 - (y + 0.5) / h);
      for (var x = 0; x < w; x++) {
        var rgb = rgbFromHsv(c.h, (x + 0.5) / w, v), at = (y * w + x) * 4;
        d[at] = byteFromLinear(rgb[0]);
        d[at + 1] = byteFromLinear(rgb[1]);
        d[at + 2] = byteFromLinear(rgb[2]);
        d[at + 3] = 255;
      }
    }
    g.putImageData(image, 0, 0);
    var picked = bytesOf(c);
    g.beginPath();
    g.arc(c.s * w, (1 - c.v) * h, w / 12, 0, 2 * Math.PI);
    g.fillStyle = hexOf(picked);
    g.fill();
    g.lineWidth = ratio;
    g.strokeStyle = contrastOn(picked);
    g.stroke();
  }
  // Paints a picker's bar of hue and the triangle egui points at the hue
  // with from below.
  function paintHue(canvas, c) {
    var ratio = window.devicePixelRatio || 1, w = Math.round(pickerWidth * ratio), h = Math.round(cssPoints('--interact-h') * ratio);
    canvas.width = w;
    canvas.height = h;
    var g = canvas.getContext('2d');
    for (var x = 0; x < w; x++) {
      g.fillStyle = hexOf(rgbFromHsv((x + 0.5) / w, 1, 1).map(byteFromLinear));
      g.fillRect(x, 0, 1, h);
    }
    var at = c.h * w, r = h / 4, hue = rgbFromHsv(c.h, 1, 1).map(byteFromLinear);
    g.beginPath();
    g.moveTo(at, h / 2);
    g.lineTo(at + r, h);
    g.lineTo(at - r, h);
    g.closePath();
    g.fillStyle = hexOf(hue);
    g.fill();
    g.lineWidth = ratio;
    g.strokeStyle = contrastOn(hue);
    g.stroke();
  }
  // Copies text to the clipboard, reporting it when neither way could; the
  // text copied is the window's last copy (lastCopied).
  function copyText(text) {
    var byCommand = function () {
      var t = el('textarea');
      t.value = text;
      t.style.cssText = 'position: fixed; opacity: 0;';
      document.body.appendChild(t);
      t.select();
      var done = document.execCommand('copy');
      t.remove();
      if (done) lastCopied = text;
      return done;
    };
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { lastCopied = text; }, function (e) {
        if (!byCommand()) post({ t: 'error', text: 'the color could not be copied: ' + e.message });
      });
    } else if (!byCommand()) {
      post({ t: 'error', text: 'the color could not be copied: this engine has no clipboard to write' });
    }
  }
  // egui's color picker for a color chooser, under its swatch: a row with
  // the button switching every picker between bytes and linear fractions,
  // the button copying the channels, and the red, green and blue drag
  // values; the color; the square of saturation and value; and the bar of
  // hue. A change raises Change at once, as dragging in egui's does.
  function openPicker(swatch, node) {
    var kept = pickerColors[node.id];
    var c = kept && kept.hex === values[node.id] ? { h: kept.h, s: kept.s, v: kept.v } : colorOfBytes(bytesOfHex(values[node.id]));
    var box = el('div', 'popup picker');
    box.setAttribute('role', 'dialog');
    var row = el('div', 'row');
    var mode = el('button', 'btn frame', channelBytes ? 'U8' : 'F');
    mode.onclick = function () { channelBytes = !channelBytes; openPicker(swatch, node); };
    var copy = el('button', 'btn frame', '📋');
    copy.setAttribute('aria-label', 'Copy the color');
    copy.onclick = function () { copyText(channelBytes ? bytesOf(c).join(', ') : linearOf(c).map(function (n) { return n.toFixed(3); }).join(', ')); };
    row.appendChild(mode);
    row.appendChild(copy);
    var shown = el('div', 'swatch-wide');
    var square = el('canvas', 'sv'), bar = el('canvas', 'hue');
    var channels = [];
    var take = function (next, from) {
      c = next;
      var picked = bytesOf(c), hex = hexOf(picked);
      pickerColors[node.id] = { hex: hex, h: c.h, s: c.s, v: c.v };
      shown.style.background = hex;
      swatch.style.setProperty('--chosen', hex);
      swatch.setAttribute('aria-label', hex);
      channels.forEach(function (field, i) { if (i !== from) field.setValue(channelBytes ? picked[i] : linearOf(c)[i]); });
      paintSquare(square, c);
      paintHue(bar, c);
      if (hex !== values[node.id]) changed(node, hex);
    };
    ['R ', 'G ', 'B '].forEach(function (prefix, i) {
      var field = channelBytes
        ? dragValue({ value: bytesOf(c)[i], range: [0, 255], speed: 0.5, decimals: 0, prefix: prefix, enabled: true, set: function (n) { var b = bytesOf(c); b[i] = Math.round(n); take(colorOfBytes(b), i); return b[i]; } })
        : dragValue({ value: linearOf(c)[i], range: [0, 1], speed: 0.003, decimals: 3, prefix: prefix, enabled: true, format: function (n) { return n.toFixed(3); }, set: function (n) { var l = linearOf(c); l[i] = n; take(colorOfLinear(l), i); return n; } });
      channels.push(field);
      row.appendChild(field);
    });
    var follow = function (canvas, place) {
      canvas.addEventListener('pointerdown', function (e) {
        if (e.button !== 0) return;
        canvas.setPointerCapture(e.pointerId);
        place(e);
        canvas.onpointermove = place;
      });
      var stop = function () { canvas.onpointermove = null; };
      canvas.addEventListener('pointerup', stop);
      canvas.addEventListener('pointercancel', stop);
    };
    var within01 = function (n) { return Math.min(Math.max(n, 0), 1); };
    follow(square, function (e) { var r = square.getBoundingClientRect(); take({ h: c.h, s: within01((e.clientX - r.left) / r.width), v: within01(1 - (e.clientY - r.top) / r.height) }, -1); });
    follow(bar, function (e) { var r = bar.getBoundingClientRect(); take({ h: within01((e.clientX - r.left) / r.width), s: c.s, v: c.v }, -1); });
    shown.style.background = hexOf(bytesOf(c));
    paintSquare(square, c);
    paintHue(bar, c);
    box.appendChild(row);
    box.appendChild(shown);
    box.appendChild(square);
    box.appendChild(bar);
    openPopup(swatch, box, node, false);
  }

  var minuteForm = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/;
  function padded(n, width) { var s = String(n); while (s.length < width) s = '0' + s; return s; }
  // A minute written YYYY-MM-DDTHH:MM moved into `low` to `high`, to the
  // nearer end when outside; the written form orders as the minutes do.
  function within(t, low, high) { return low && t < low ? low : high && t > high ? high : t; }
  // A day of the proleptic Gregorian calendar, as a date at midnight UTC.
  function utcDay(year, month, day) { var d = new Date(0); d.setUTCFullYear(year, month - 1, day); return d; }
  function daysIn(year, month) { return utcDay(year, month + 1, 0).getUTCDate(); }
  // How many days of the week come before the month's first, from Monday.
  function firstWeekday(year, month) { return (utcDay(year, month, 1).getUTCDay() + 6) % 7; }
  // A date chooser, as the egui renderer draws one: a field naming the day
  // chosen, a drop-down in a text field's look, which opens a calendar of a
  // month under it, and once a day is chosen its hour and minute beside it
  // as drag values in the same look; under a stretched layout the day takes
  // the room the hour and minute leave. A minute outside the chooser's
  // minimum and maximum is moved to the nearer of them.
  function drawDate(node, p) {
    ['minimum', 'maximum'].forEach(function (key) {
      if (p[key] !== undefined && p[key] !== null && (typeof p[key] !== 'string' || !minuteForm.test(p[key]))) throw new Error(key + ' must be a date written YYYY-MM-DDTHH:MM');
    });
    var v = values[node.id];
    if (v !== null && v !== undefined && (typeof v !== 'string' || !minuteForm.test(v))) throw new Error('value must be a date written YYYY-MM-DDTHH:MM, not ' + JSON.stringify(v));
    var enabled = flag(p, 'enabled', true);
    var low = typeof p.minimum === 'string' ? p.minimum : null, high = typeof p.maximum === 'string' ? p.maximum : null;
    var box = el('span', 'row date' + (enabled ? '' : ' off'));
    var day = comboButton(v ? v.slice(0, 10) : 'Choose a date', enabled, false);
    day.dataset.node = node.id + '-day';
    day.setAttribute('aria-haspopup', 'dialog');
    day.onclick = function (e) {
      if (popup && popup.anchor === day) { closePopup(); return; }
      openCalendar(day, node, low, high, e.detail === 0);
    };
    box.appendChild(day);
    if (v) {
      var part = function (at, n) {
        var t = within(values[node.id].slice(0, at) + padded(n, 2) + values[node.id].slice(at + 2), low, high);
        if (t !== values[node.id]) changed(node, t);
        return Number(t.slice(at, at + 2));
      };
      var two = function (n) { return padded(n, 2); };
      box.appendChild(dragValue({ node: node.id + '-hour', value: Number(v.slice(11, 13)), range: [0, 23], speed: 1, decimals: 0, format: two, well: true, enabled: enabled, set: function (n) { return part(11, n); } }));
      box.appendChild(el('span', 'label', ':'));
      box.appendChild(dragValue({ node: node.id + '-minute', value: Number(v.slice(14, 16)), range: [0, 59], speed: 1, decimals: 0, format: two, well: true, enabled: enabled, set: function (n) { return part(14, n); } }));
    }
    return box;
  }
  // The calendar of a date chooser, under its button: `<`, the month, `>`,
  // then the days of the month under Mo to Su, those outside the minimum
  // and maximum disabled. Choosing a day leaves the hour and minute chosen
  // before, or midnight, and closes it. Opened from the keyboard, the day
  // chosen or the first allowed takes the focus.
  function openCalendar(button, node, low, high, fromKeys) {
    var box = el('div', 'popup');
    box.setAttribute('role', 'dialog');
    var pick = function (stamp) {
      var v = values[node.id];
      var t = within(stamp + (v ? v.slice(10) : 'T00:00'), low, high);
      closePopup();
      if (t !== values[node.id]) changed(node, t);
      redraw(node);
    };
    var fill = function () {
      var v = values[node.id], now = new Date();
      var shown = calendarMonths[node.id] || (v ? [Number(v.slice(0, 4)), Number(v.slice(5, 7))] : low ? [Number(low.slice(0, 4)), Number(low.slice(5, 7))] : [now.getUTCFullYear(), now.getUTCMonth() + 1]);
      calendarMonths[node.id] = shown;
      var year = shown[0], month = shown[1], prefix = padded(year, 4) + '-' + padded(month, 2);
      var head = el('div', 'row');
      var back = el('button', 'btn frame', '<');
      back.disabled = !(year > 1 || month > 1);
      back.onclick = function () { calendarMonths[node.id] = month === 1 ? [year - 1, 12] : [year, month - 1]; fill(); };
      var next = el('button', 'btn frame', '>');
      next.disabled = !(year < 9999 || month < 12);
      next.onclick = function () { calendarMonths[node.id] = month === 12 ? [year + 1, 1] : [year, month + 1]; fill(); };
      head.appendChild(back);
      head.appendChild(el('span', 'label', prefix));
      head.appendChild(next);
      var grid = el('div', 'calendar');
      grid.setAttribute('role', 'grid');
      ['Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa', 'Su'].forEach(function (name) { grid.appendChild(el('span', 'label', name)); });
      for (var gap = firstWeekday(year, month); gap > 0; gap--) grid.appendChild(el('span'));
      var chosen = v && v.slice(0, 7) === prefix ? Number(v.slice(8, 10)) : null, first = null;
      for (var d = 1; d <= daysIn(year, month); d++) {
        var stamp = prefix + '-' + padded(d, 2);
        var b = el('button', 'btn frame selectable' + (chosen === d ? ' picked' : ''), String(d));
        b.disabled = !((!low || stamp + 'T23:59' >= low) && (!high || stamp + 'T00:00' <= high));
        b.setAttribute('aria-selected', chosen === d ? 'true' : 'false');
        b.onclick = (function (s) { return function () { pick(s); }; })(stamp);
        if (!b.disabled && (first === null || chosen === d)) first = b;
        grid.appendChild(b);
      }
      box.replaceChildren(head, grid);
      return first;
    };
    var first = fill();
    openPopup(button, box, node, false);
    if (fromKeys && first) first.focus();
  }

  // A file chooser: its paths, or what it has none of, beside the button
  // that requests the system's dialog from the helper, which waits while one
  // is open.
  function drawFile(node, p) {
    var paths = values[node.id];
    if (!Array.isArray(paths) || paths.some(function (x) { return typeof x !== 'string'; })) throw new Error('paths must be a list of text');
    var enabled = flag(p, 'enabled', true);
    var box = el('span', 'file' + (enabled ? '' : ' off'));
    var shown = paths.length === 0 ? (flag(p, 'folder', false) ? 'No folder chosen' : 'No file chosen') : paths.length === 1 ? paths[0] : paths[0] + ' and ' + (paths.length - 1) + ' more';
    box.appendChild(el('span', 'label paths', shown));
    var b = el('button', 'btn frame', 'Browse…');
    b.disabled = !enabled || browsing.has(node.id);
    b.dataset.node = node.id + '-browse';
    b.onclick = function () {
      browsing.add(node.id);
      b.disabled = true;
      post({ t: 'browse', node: node.id, paths: paths });
    };
    box.appendChild(b);
    return box;
  }

  // The color a chart gives its series `i`, as egui_plot gives it: hues
  // the golden ratio apart, saturation 0.85 and value 0.5 in linear light,
  // written as sRGB.
  function seriesColor(i) {
    var h = (i * 0.6180339887498949) % 1, s = 0.85, v = 0.5;
    var channel = function (n) { var k = (n + h * 6) % 6; return v - v * s * Math.max(0, Math.min(k, 4 - k, 1)); };
    var srgb = function (c) { return Math.round(255 * (c <= 0.0031308 ? 12.92 * c : 1.055 * Math.pow(c, 1 / 2.4) - 0.055)); };
    return 'rgb(' + srgb(channel(5)) + ', ' + srgb(channel(3)) + ', ' + srgb(channel(1)) + ')';
  }
  // The values under chart `u`'s pointer, beside it, as egui_plot writes
  // them: the series, X and Y of the point nearest the pointer within five
  // pixels, egui's reach, or else the pointer's own X and Y.
  function chartTip(u, c) {
    var tip = u.over.querySelector('.chart-tip');
    if (!tip) { tip = el('div', 'chart-tip'); u.over.appendChild(tip); }
    var left = u.cursor.left, top = u.cursor.top;
    if (left === null || left === undefined || left < 0) { tip.style.display = 'none'; return; }
    var text = pwrsLayout.xText(u.posToVal(left, 'x'), c.dates) + '\n' + String(u.posToVal(top, 'y'));
    var idx = u.cursor.idx, near = null;
    if (idx !== null && idx !== undefined) {
      c.series.forEach(function (s, si) {
        var v = u.data[si + 1][idx];
        if (v === null || v === undefined) return;
        var d = Math.hypot(u.valToPos(u.data[0][idx], 'x') - left, u.valToPos(v, 'y') - top);
        if (d <= 5 && (!near || d < near.d)) near = { d: d, name: s.name, x: u.data[0][idx], y: v };
      });
    }
    if (near) text = near.name + '\n' + pwrsLayout.xText(near.x, c.dates) + '\n' + String(near.y);
    tip.textContent = text;
    tip.style.display = 'block';
    tip.style.left = (left + 12) + 'px';
    tip.style.top = (top + 12) + 'px';
  }
  // The gap between an X axis's labels, in CSS pixels, at or below which
  // egui_plot writes none.
  var DATE_LABEL_GAP = 60;
  // How wide `text` is in chart `u`'s X axis font, in CSS pixels: uPlot
  // stores that font as its text at the screen's scale, that size, and the
  // size in CSS pixels.
  function axisTextWidth(u, text) {
    var font = u.axes[0].font;
    u.ctx.save();
    u.ctx.font = font[0];
    var wide = u.ctx.measureText(text).width * font[2] / font[1];
    u.ctx.restore();
    return wide;
  }

  // A chart drawn by uPlot over one X shared by its series: its points in
  // X order, each series with its Y in its own rows. A person drags to
  // pan, drags with the secondary button to zoom to a box, zooms with Ctrl
  // and the wheel, double-clicks to go back, reads the values under the
  // pointer in the legend, and clicks a point to raise Select, which a bar
  // takes anywhere on it and any other point within five pixels, egui's
  // reach for a pointer.
  function drawChart(node, p) {
    var c = p.chart;
    if (!c || typeof c !== 'object') throw new Error('the chart came without its data');
    if (c.problem) throw new Error(c.problem);
    if (typeof uPlot !== 'function') {
      var raised = window.pwrs.__loadErrors;
      throw new Error('the chart library did not load' + (raised.length ? ': ' + raised[0] : ''));
    }
    var h = optNumber(p, 'height');
    if (h !== null && !(h > 0)) throw new Error('height must be above 0 points, not ' + h);
    if (charts[node.id]) { charts[node.id].observer.disconnect(); charts[node.id].plot.destroy(); delete charts[node.id]; }
    var rows = [];
    c.series.forEach(function (s, si) { s.points.forEach(function (pt, pi) { rows.push({ x: pt[0], s: si, i: pi }); }); });
    rows.sort(function (a, b) { return a.x - b.x || a.s - b.s || a.i - b.i; });
    var data = [rows.map(function (r) { return r.x; })].concat(c.series.map(function (s, si) {
      var none = s.kind === 'bar' ? undefined : null;
      return rows.map(function (r) { return r.s === si ? s.points[r.i][1] : none; });
    }));
    // An app's chart: on the window's own background, hairlines across in the
    // line color at least 40 points apart and none down, no tick marks, and
    // the axes' text in the weak text color, as the egui renderer draws it.
    var look = getComputedStyle(document.documentElement);
    var weak = look.getPropertyValue('--weak').trim(), line = look.getPropertyValue('--noninteractive-stroke').trim();
    var xAxis = { stroke: weak, grid: { show: false }, ticks: { show: false }, label: typeof p.x_label === 'string' ? p.x_label : undefined };
    if (c.dates) {
      // A date axis takes the finest of uPlot's steps whose lines stand more
      // than DATE_LABEL_GAP apart, with room for its widest label, as
      // egui_plot labels an axis. The gap returned is just under that
      // step's, so uPlot, which takes the first step at least that far
      // apart, takes that one. A label still too wide for the gap, a date
      // at midnight, is left out, as egui_plot leaves it out.
      xAxis.space = function (u, axis, min, max, dim) {
        var perSecond = dim / (max - min), incrs = u.axes[axis].incrs(u, axis, min, max, dim, DATE_LABEL_GAP);
        for (var i = 0; i < incrs.length; i++) {
          var gap = incrs[i] * perSecond;
          if (gap > DATE_LABEL_GAP && gap >= axisTextWidth(u, pwrsLayout.dateSample(incrs[i]))) return gap * 0.999;
        }
        return DATE_LABEL_GAP;
      };
      xAxis.values = function (u, splits, axis, space, step) {
        return splits.map(function (v) { var t = pwrsLayout.dateTick(v, step); return axisTextWidth(u, t) > space ? '' : t; });
      };
    }
    var series = [{ label: c.dates ? 'time' : 'x', value: function (u, v) { return v === null || v === undefined ? '' : pwrsLayout.xText(v, c.dates); } }];
    c.series.forEach(function (s, i) {
      var color = seriesColor(i);
      var o = { label: s.name, stroke: color, width: 2, spanGaps: true };
      if (s.kind === 'points') { o.paths = function () { return null; }; o.points = { show: true, size: 8, fill: color }; }
      if (s.kind === 'bar') { o.paths = uPlot.paths.bars({ size: [0.5, Infinity] }); o.fill = color; o.width = 0; o.fillTo = 0; o.points = { show: false }; }
      series.push(o);
    });
    var box = el('div', 'chart');
    box.dataset.node = node.id;
    // A chart given a height is that tall while its window is measured.
    if (h !== null) box.dataset.height = h;
    var home = null;
    var u = new uPlot({
      width: 400,
      height: h === null ? 240 : h,
      scales: { x: { time: c.dates } },
      tzDate: function (ts) { return uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'); },
      axes: [xAxis, { stroke: weak, grid: { stroke: line, width: 1 }, ticks: { show: false }, space: 40, label: typeof p.y_label === 'string' ? p.y_label : undefined }],
      cursor: { drag: { x: false, y: false } },
      legend: { show: c.series.length > 1, live: false },
      series: series,
      hooks: {
        ready: [function (self) {
          home = { x: [self.scales.x.min, self.scales.x.max], y: [self.scales.y.min, self.scales.y.max] };
          self.scales.y.auto = function () { return false; };
        }],
        setCursor: [function (self) { chartTip(self, c); }]
      }
    }, data, box);
    // The plot takes the box's width, and with no height of its own the
    // height the window leaves it, less its legend's.
    var observer = new ResizeObserver(function () {
      var legend = box.querySelector('.u-legend');
      var high = h === null ? box.clientHeight - (legend ? legend.offsetHeight : 0) : h;
      if (box.clientWidth > 0 && high > 0) u.setSize({ width: box.clientWidth, height: high });
    });
    observer.observe(box);
    charts[node.id] = { plot: u, observer: observer };
    var scaleTo = function (x, y) { u.batch(function () { u.setScale('x', { min: x[0], max: x[1] }); u.setScale('y', { min: y[0], max: y[1] }); }); };
    // The plot is worked as egui_plot works it, by egui's drag and click
    // rules for the window (engage). Once a press of the primary button is a
    // drag, the plot pans with each move from the one that made it one. Once
    // one of the secondary button is, its box runs from where the pointer was
    // then to where it is, and the plot zooms to the box as the drag ends,
    // where the box has an area. A primary click raises Select on the point
    // under the pointer, and a primary double-click shows the whole plot
    // again.
    var over = u.over, drag = null, prev = null;
    var local = function (e) { var r = over.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    var steer = function (e) {
      var at = local(e), dragging = engaged.dragged === over, before = prev || at;
      prev = at;
      if (dragging && !drag) {
        drag = { slot: pointerNow.down[0] ? 0 : pointerNow.down[1] ? 1 : null, from: at, last: before };
      }
      if (drag && drag.slot === 0 && dragging) {
        var dx = at[0] - drag.last[0], dy = at[1] - drag.last[1], sx = [u.scales.x.min, u.scales.x.max], sy = [u.scales.y.min, u.scales.y.max];
        var ix = (sx[1] - sx[0]) * dx / over.clientWidth, iy = (sy[1] - sy[0]) * dy / over.clientHeight;
        if (dx !== 0 || dy !== 0) scaleTo([sx[0] - ix, sx[1] - ix], [sy[0] + iy, sy[1] + iy]);
      } else if (drag && drag.slot === 1 && dragging) {
        u.setSelect({ left: Math.min(drag.from[0], at[0]), top: Math.min(drag.from[1], at[1]), width: Math.abs(at[0] - drag.from[0]), height: Math.abs(at[1] - drag.from[1]) }, false);
      }
      if (drag) drag.last = at;
      if (drag && !dragging) {
        var d = drag;
        drag = null;
        if (d.slot === 1) {
          var left = Math.min(d.from[0], at[0]), right = Math.max(d.from[0], at[0]), top = Math.min(d.from[1], at[1]), bottom = Math.max(d.from[1], at[1]);
          if (right > left && bottom > top) scaleTo([u.posToVal(left, 'x'), u.posToVal(right, 'x')], [u.posToVal(bottom, 'y'), u.posToVal(top, 'y')]);
          u.setSelect({ left: 0, top: 0, width: 0, height: 0 }, false);
        }
      }
    };
    over.addEventListener('pointerdown', function (e) {
      over.setPointerCapture(e.pointerId);
      steer(e);
    });
    over.addEventListener('pointermove', steer);
    over.addEventListener('pointerup', function (e) {
      steer(e);
      var click = e.button === 0 && engaged.clicked === over ? clickOf(e, 0) : null;
      if (!click) return;
      var at = local(e);
      clickAt(at[0], at[1]);
      if (click.count === 2 && home) scaleTo(home.x, home.y);
    });
    over.addEventListener('pointercancel', steer);
    over.addEventListener('wheel', function (e) {
      if (!e.ctrlKey) return;
      e.preventDefault();
      var f = Math.exp(e.deltaY * 0.002), cx = u.posToVal(e.offsetX, 'x'), cy = u.posToVal(e.offsetY, 'y');
      scaleTo([cx - (cx - u.scales.x.min) * f, cx + (u.scales.x.max - cx) * f], [cy - (cy - u.scales.y.min) * f, cy + (u.scales.y.max - cy) * f]);
    }, { passive: false });
    // A bar's half width in pixels, as uPlot draws a bar: half the gap
    // between the series' nearest two points, or the whole plot's width
    // for a series of one.
    function barHalf(s) {
      var xs = s.points.map(function (pt) { return pt[0]; }).sort(function (a, b) { return a - b; });
      var gap = Infinity, wide = over.clientWidth;
      for (var i = 1; i < xs.length; i++) { if (xs[i] - xs[i - 1] < gap) { gap = xs[i] - xs[i - 1]; wide = Math.abs(u.valToPos(xs[i], 'x') - u.valToPos(xs[i - 1], 'x')); } }
      return wide * 0.5 / 2;
    }
    function clickAt(px, py) {
      var best = null;
      c.series.forEach(function (s, si) {
        var half = s.kind === 'bar' ? barHalf(s) : 0;
        s.points.forEach(function (pt, pi) {
          var x = u.valToPos(pt[0], 'x'), y = u.valToPos(pt[1], 'y');
          if (s.kind === 'bar') {
            var base = u.valToPos(0, 'y');
            if (!best && px >= x - half && px <= x + half && py >= Math.min(y, base) && py <= Math.max(y, base)) best = { d: -1, si: si, pi: pi };
            return;
          }
          var d = Math.hypot(px - x, py - y);
          if (d <= 5 && (!best || (best.d >= 0 && d < best.d))) best = { d: d, si: si, pi: pi };
        });
      });
      if (!best) return;
      var hit = c.series[best.si], at = hit.points[best.pi];
      emit(node, 'Select', { series: hit.name, x: at[2], y: at[1] });
    }
    return box;
  }

  // How the page draws its GpuSurfaces: with WebGPU, on a device requested
  // when a surface first needs one, with every feature and limit its
  // adapter has, as the egui renderer opens its device, and requested again
  // after a loss; or, on an engine whose WebGPU offers no adapter,
  // with WebGL 2, in a context of each surface's own. `mode` is settled
  // the first time a surface needs it.
  var gpu = { mode: null, moding: null, adapter: null, device: null, asking: null, building: 0 };
  // The engine the page runs on, its name and version, which the page names
  // when it refuses what the engine cannot draw.
  var engineName = 'this engine';
  // The copies of the egui renderer's table objects the page's surfaces
  // draw from, by the name each is fetched under: the version taken,
  // the resource that named it last, its object on the device, and its
  // takes, which run one after another.
  var copies = {};
  // Each GpuSurface by node id, as surfaceOf makes it.
  var surfaces = {};
  // Whether an animation frame is requested, in which the surfaces waiting
  // for a frame draw.
  var framing = false;
  // The texture formats WebGPU binds as storage without a feature, which a
  // texture's copy is made able to be bound as.
  var storable = ['rgba8unorm', 'rgba16float', 'r32float', 'rg32float', 'rgba32float', 'r32uint', 'r32sint'];
  var identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  // What a surface clears to until the session gives it draws: the
  // window's own background in the theme the page shows, as the egui
  // renderer's does.
  function unset() {
    var hex = themeColor('--panel');
    return [1, 3, 5].map(function (i) { return parseInt(hex.slice(i, i + 2), 16) / 255; }).concat([1]);
  }

  // An error a GPU call is refused with: its id, as the GPU layer names its
  // errors, and its text.
  function gpuError(id, text) {
    var e = new Error(text);
    e.id = id;
    return e;
  }

  // GpuSurface `id`'s state: what the session gave it and its plan, its
  // builds, which run one after another, its node, box and canvas, its
  // WebGPU context or its WebGL 2 context with the copies and targets that
  // context has, the context's return while the engine has it, and how
  // many times it has come back, the targets a multisampled or depth-tested
  // WebGPU plan draws into, when it was first laid out and last drawn, the
  // pointer over it in the page's points, whether it waits for a frame, and
  // the problem last reported; the pointer events its node requests,
  // whether it takes keys, the buttons pressed over it, the pointer's
  // latest place and the last move reported; and its camera, the drag
  // moving it, and whether the camera moved since the helper was told.
  function surfaceOf(id) {
    return surfaces[id] || (surfaces[id] = {
      id: id, given: null, plan: null, builds: Promise.resolve(), node: null, box: null, canvas: null, context: null, configured: null,
      gl: null, glCopies: null, glTargets: null, glBack: null, glRound: 0,
      observer: null, targets: null, started: null, last: null, hover: null, dirty: true, reported: null,
      wanted: { down: false, up: false, moves: false, wheel: false }, keys: false, pressed: [false, false, false], latest: null, lastMove: null, moveDue: false,
      camera: null, held: null, cameraMoved: false
    });
  }

  // Reports a surface's problem, once until another takes its place, as the
  // egui renderer reports what its frames meet.
  function surfaceProblem(s, text) {
    if (s.reported === text) return;
    s.reported = text;
    var node = find(tree, s.id);
    post({ t: 'error', context: 'GPU', text: 'GpuSurface ' + (node && node.name ? node.name : s.id) + ': ' + text });
  }

  // Whether the page draws with 'webgpu' or 'webgl2': WebGPU when the
  // engine's WebGPU offers an adapter, which the page's device is then
  // requested from, and otherwise WebGL 2, refused when the engine has
  // neither. The helper is told which once it is settled.
  function gpuMode() {
    if (gpu.mode) return Promise.resolve(gpu.mode);
    if (gpu.moding) return gpu.moding;
    var webgl = function () {
      var probe = document.createElement('canvas').getContext('webgl2');
      if (!probe) throw gpuError('GuiGpuNoWebGpu', engineName + ' has neither WebGPU nor WebGL 2, so it draws no GpuSurface; a window opened with -Renderer Egui draws it');
      var lose = probe.getExtension('WEBGL_lose_context');
      if (lose) lose.loseContext();
      return 'webgl2';
    };
    var asked = navigator.gpu ? navigator.gpu.requestAdapter().then(function (adapter) {
      if (!adapter) return webgl();
      gpu.adapter = adapter;
      return 'webgpu';
    }) : Promise.resolve().then(webgl);
    gpu.moding = asked.then(function (mode) {
      gpu.mode = mode;
      gpu.moding = null;
      post({ t: 'graphics', graphics: mode === 'webgpu' ? 'WebGPU' : 'WebGL2' });
      return mode;
    }, function (err) {
      gpu.moding = null;
      throw err;
    });
    return gpu.moding;
  }

  // The page's device, requested from the engine's WebGPU the first time.
  function gpuDevice() {
    if (gpu.device) return Promise.resolve(gpu.device);
    if (gpu.asking) return gpu.asking;
    if (!navigator.gpu) return Promise.reject(gpuError('GuiGpuNoWebGpu', engineName + ' has no WebGPU'));
    gpu.asking = (gpu.adapter ? Promise.resolve(gpu.adapter) : navigator.gpu.requestAdapter()).then(function (adapter) {
      if (!adapter) throw gpuError('GuiGpuNoWebGpu', engineName + "'s WebGPU offers no adapter");
      var limits = {};
      for (var limit in adapter.limits) limits[limit] = adapter.limits[limit];
      return adapter.requestDevice({ requiredFeatures: Array.from(adapter.features), requiredLimits: limits });
    }).then(function (device) {
      gpu.device = device;
      gpu.asking = null;
      device.addEventListener('uncapturederror', function (e) { post({ t: 'error', context: 'GPU', text: "the page's WebGPU device: " + e.error.message }); });
      device.lost.then(function (info) { deviceLost(device, info); });
      return device;
    }, function (err) {
      gpu.asking = null;
      throw err.id ? err : gpuError('GuiGpuNoWebGpu', engineName + "'s WebGPU gives no device: " + err.message);
    });
    return gpu.asking;
  }

  // The page's device is lost: the loss is reported, and every surface the
  // session gave draws builds them again on a new device, its copies taken
  // again, as the egui renderer goes on with a new device.
  function deviceLost(device, info) {
    if (gpu.device !== device) return;
    gpu.device = null;
    post({ t: 'error', context: 'GPU', text: "the page's WebGPU device was lost (" + info.reason + '): ' + info.message });
    Object.keys(surfaces).forEach(function (id) {
      var s = surfaces[id];
      s.plan = null;
      s.targets = null;
      s.configured = null;
      if (s.given) give(s, s.given, null);
    });
  }

  // The bytes of the copy the helper stores at `name`, of `version` or a
  // later one.
  function copyBytes(name, version) {
    return fetch('gpu/' + name + '?v=' + version).then(function (response) {
      if (response.ok) return response.arrayBuffer();
      return response.text().then(function (text) { throw gpuError('GuiGpuNoCopy', text); });
    });
  }

  // Takes the copy resource `r` names onto `device`, at its version or a
  // later one, unless the copy there is that new already.
  function take(device, r) {
    var name = r.buffer || r.texture || r.sampler;
    var c = copies[name] || (copies[name] = { version: 0, resource: r, device: null, object: null, view: null, takes: Promise.resolve() });
    var taken = c.takes.then(function () {
      if (c.device === device && c.object && c.version >= r.version) return c;
      return (r.sampler ? Promise.resolve(null) : copyBytes(name, r.version)).then(function (bytes) {
        put(device, c, r, bytes);
        return c;
      });
    });
    // The next take of the copy waits for this one however it went; this
    // one's caller gets the outcome.
    c.takes = taken.then(function () {}, function () {});
    return taken;
  }

  // Makes copy `c` on `device` from resource `r` and its bytes, or writes
  // the bytes into the object it has there of the same size and format,
  // which the plans drawing from it read from then on. An object of
  // another size or format is made anew, and the plans with the one before
  // go on using it until they are built again.
  function put(device, c, r, bytes) {
    var name = r.buffer || r.texture || r.sampler;
    var fresh = c.device !== device || !c.object;
    if (r.sampler) {
      c.object = device.createSampler({ magFilter: r.filter, minFilter: r.filter, mipmapFilter: r.filter, addressModeU: r.address, addressModeV: r.address, addressModeW: r.address });
    } else if (r.buffer) {
      if (bytes.byteLength !== r.length) throw gpuError('GuiGpuNoCopy', name + ' came with ' + bytes.byteLength + ' bytes for a buffer of ' + r.length + ' bytes');
      var size = Math.max(4, Math.ceil(r.length / 4) * 4);
      if (fresh || c.object.size !== size) c.object = device.createBuffer({ size: size, usage: anyBufferUse() });
      var padded = new Uint8Array(size);
      padded.set(new Uint8Array(bytes));
      device.queue.writeBuffer(c.object, 0, padded);
    } else {
      var rowBytes = r.width * r.texelBytes;
      if (bytes.byteLength !== rowBytes * r.height) throw gpuError('GuiGpuNoCopy', name + ' came with ' + bytes.byteLength + ' bytes, and its ' + r.width + ' by ' + r.height + ' texels of ' + r.format + ' take ' + rowBytes * r.height);
      if (fresh || c.object.width !== r.width || c.object.height !== r.height || c.object.format !== r.format) {
        var usage = GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_SRC | GPUTextureUsage.COPY_DST;
        if (storable.indexOf(r.format) >= 0) usage |= GPUTextureUsage.STORAGE_BINDING;
        c.object = device.createTexture({ size: [r.width, r.height], format: r.format, usage: usage });
        c.view = c.object.createView();
      }
      device.queue.writeTexture({ texture: c.object }, bytes, { bytesPerRow: rowBytes, rowsPerImage: r.height }, [r.width, r.height]);
    }
    c.device = device;
    c.version = r.version;
    c.resource = r;
  }

  // Every use a buffer of the egui renderer's table can have but mapping,
  // which its copy has too.
  function anyBufferUse() {
    return GPUBufferUsage.VERTEX | GPUBufferUsage.INDEX | GPUBufferUsage.UNIFORM | GPUBufferUsage.STORAGE | GPUBufferUsage.INDIRECT | GPUBufferUsage.COPY_SRC | GPUBufferUsage.COPY_DST;
  }

  // Lets go of the copies no surface's plan draws from, once no build is
  // taking any, and of the copies each surface's WebGL 2 context has that
  // its own plan does not draw from.
  function sweepCopies() {
    if (gpu.building > 0) return;
    var used = {};
    Object.keys(surfaces).forEach(function (id) {
      var s = surfaces[id];
      var drawn = s.plan ? s.plan.names : [];
      drawn.forEach(function (name) { used[name] = true; });
      if (!s.glCopies || !s.gl || s.gl.isContextLost()) return;
      Object.keys(s.glCopies).forEach(function (name) {
        if (drawn.indexOf(name) >= 0) return;
        Object.keys(s.glCopies[name]).forEach(function (form) {
          var o = s.glCopies[name][form].object;
          if (form === 'texture') s.gl.deleteTexture(o); else if (form === 'sampler') s.gl.deleteSampler(o); else s.gl.deleteBuffer(o);
        });
        delete s.glCopies[name];
      });
    });
    Object.keys(copies).forEach(function (name) {
      if (used[name]) return;
      var c = copies[name];
      if (c.object && !c.resource.sampler) c.object.destroy();
      delete copies[name];
    });
  }

  // Whether WebGPU blend state `blend` reads the blend constant.
  function readsConstant(blend) {
    if (!blend) return false;
    return [blend.color.srcFactor, blend.color.dstFactor, blend.alpha.srcFactor, blend.alpha.dstFactor].some(function (f) { return f === 'constant' || f === 'one-minus-constant'; });
  }

  // The format of depth attachment `target`, a spec's depthTarget, as
  // WebGPU names it, or null for none.
  function depthFormatOf(target) {
    return target ? target.format : null;
  }

  // Why the egui renderer would refuse `given`'s draws before recording
  // them: pipelines of other samples or depth formats, which a surface's
  // draws share, a blend constant given or missing against what the blend
  // reads, or a depth or stencil clear for an attachment that holds none;
  // null when it would not.
  function drawsRefused(given) {
    var first = given.draws[0];
    for (var at = 1; at < given.draws.length; at++) {
      var spec = given.draws[at].spec;
      if (spec.sampleCount !== first.spec.sampleCount || depthFormatOf(spec.depthTarget) !== depthFormatOf(first.spec.depthTarget)) {
        var depthOf = function (t) { return t ? t.name + ' depth' : 'no depth'; };
        var text = 'draw ' + at + "'s pipeline draws with " + spec.sampleCount + ' samples and ' + depthOf(spec.depthTarget) + ", and draw 0's with " + first.spec.sampleCount + ' and ' + depthOf(first.spec.depthTarget) + "; a surface's draws share its attachments";
        return gpuError('GuiGpuSurfaceTargets', text);
      }
    }
    for (var d = 0; d < given.draws.length; d++) {
      var reads = readsConstant(given.draws[d].spec.blend), set = given.draws[d].blendConstant !== null;
      if (reads && !set) return gpuError('WgpuBlendConstant', 'the blend reads a constant: give the draw a blend_constant');
      if (!reads && set) return gpuError('WgpuBlendConstant', "a draw's blend_constant sets the constant a blend reads, and this blend reads none");
    }
    var target = first ? first.spec.depthTarget : null;
    var lacking = function (aspect) { return target ? "the draws' depth target, " + target.name + ', has no ' + aspect + ' aspect' : "the draws' pipelines have no depth target"; };
    if (given.clearDepth !== null && !(target && target.depth)) return gpuError('GuiGpuClearDepth', '-ClearDepth sets the depth the depth target starts at, and ' + lacking('depth'));
    if (given.clearStencil !== null && !(target && target.stencil)) return gpuError('GuiGpuClearStencil', '-ClearStencil sets the value the stencil starts at, and ' + lacking('stencil'));
    return null;
  }

  // The depth and stencil a plan of `given` clears its depth attachment
  // to: those `given` names, or 1 and 0.
  function clearsOf(given) {
    return { depth: given.clearDepth === null ? 1 : given.clearDepth, stencil: given.clearStencil === null ? 0 : given.clearStencil };
  }

  // The depth attachment of a WebGPU pass drawing `plan` on `view`: each
  // aspect its format holds cleared to the plan's depth or stencil, and
  // stored, as the egui renderer's plan clears its own.
  function depthAttachment(plan, view) {
    var attachment = { view: view };
    if (plan.depthTarget.depth) {
      attachment.depthClearValue = plan.clearDepth;
      attachment.depthLoadOp = 'clear';
      attachment.depthStoreOp = 'store';
    }
    if (plan.depthTarget.stencil) {
      attachment.stencilClearValue = plan.clearStencil;
      attachment.stencilLoadOp = 'clear';
      attachment.stencilStoreOp = 'store';
    }
    return attachment;
  }

  // `given`, a surface as the helper gives it, built on `device`: each
  // pass's and draw's pipeline, the copies they draw from, the arrays they
  // were given, and the frame uniform when one reads it; then every pass
  // and draw recorded once, unsubmitted, in an error scope, so what WebGPU
  // refuses is refused with its reason before a frame draws it, as the
  // egui renderer checks a surface's plan. Returns the plan.
  function build(device, given) {
    var stopped = drawsRefused(given);
    if (stopped) return Promise.reject(stopped);
    var names = [];
    var modules = {};
    // The shader module of `code`, once its compilation shows no error;
    // made in an error scope of its own, so a shader WebGPU refuses is
    // refused here alone.
    var shader = function (code) {
      if (modules[code]) return modules[code];
      device.pushErrorScope('validation');
      var module = device.createShaderModule({ code: code });
      var scoped = device.popErrorScope();
      modules[code] = Promise.all([module.getCompilationInfo(), scoped]).then(function (got) {
        var errors = got[0].messages.filter(function (m) { return m.type === 'error'; });
        if (errors.length) throw gpuError('GuiGpuDraw', "the page's WebGPU refuses the shader: " + errors.map(function (m) { return 'line ' + m.lineNum + ', column ' + m.linePos + ': ' + m.message; }).join('; '));
        if (got[1]) throw gpuError('GuiGpuDraw', "the page's WebGPU refuses the shader: " + got[1].message);
        return module;
      });
      return modules[code];
    };
    // The copy a resource names, taken onto the device; an array's bytes
    // become a buffer of the plan's own below.
    var copyOf = function (r) {
      if (r.missing) return Promise.reject(gpuError('GuiGpuNoCopy', 'the HTML renderer has no copy of ' + r.missing));
      if (r.bytes) return Promise.resolve(null);
      var name = r.buffer || r.texture || r.sampler;
      if (names.indexOf(name) < 0) names.push(name);
      return take(device, r);
    };
    var copiesOf = function (resources) { return Promise.all(resources.map(copyOf)); };
    var refused = function (what) {
      return function (err) { throw err.id ? err : gpuError('GuiGpuDraw', what + ': ' + err.message); };
    };
    var draws = given.draws.map(function (draw, at) {
      var spec = draw.spec;
      var strip = spec.topology === 'line-strip' || spec.topology === 'triangle-strip';
      var pipeline = shader(spec.source).then(function (module) {
        return device.createRenderPipelineAsync({
          layout: 'auto',
          vertex: {
            module: module,
            entryPoint: spec.vertexEntry || undefined,
            constants: spec.constants,
            buffers: draw.vertices.map(function (v) { return { arrayStride: v.stride, stepMode: 'vertex', attributes: [{ shaderLocation: v.location, format: v.format, offset: 0 }] }; })
          },
          fragment: { module: module, entryPoint: spec.fragmentEntry || undefined, constants: spec.constants, targets: [{ format: 'rgba8unorm', blend: spec.blend || undefined }] },
          primitive: { topology: spec.topology, cullMode: spec.cull, stripIndexFormat: strip && draw.index ? draw.index.format : undefined },
          depthStencil: spec.depthStencil || undefined,
          multisample: { count: spec.sampleCount }
        });
      }).then(null, refused('draw ' + at + "'s pipeline"));
      return Promise.all([pipeline, copiesOf(draw.vertices.map(function (v) { return v.resource; })), copiesOf(draw.bindings.map(function (b) { return b.resource; })), copiesOf(draw.index ? [draw.index.resource] : [])]);
    });
    var passes = given.passes.map(function (pass, at) {
      var pipeline = shader(pass.source).then(function (module) {
        return device.createComputePipelineAsync({ layout: 'auto', compute: { module: module, entryPoint: pass.entry, constants: pass.constants } });
      }).then(null, refused('pass ' + at + "'s pipeline"));
      return Promise.all([pipeline, copiesOf(pass.bindings.map(function (b) { return b.resource; })), copiesOf(pass.indirect ? [pass.indirect.resource] : [])]);
    });
    gpu.building++;
    var built = Promise.all([Promise.all(draws), Promise.all(passes)]).then(function (all) {
      device.pushErrorScope('validation');
      var plan;
      try {
        plan = assemble(device, given, all[0], all[1], names);
      } catch (err) {
        // The scope opened for the assembly closes with it; the error that
        // stopped it is the one to report.
        device.popErrorScope();
        throw err;
      }
      return device.popErrorScope().then(function (error) {
        if (error) throw gpuError('GuiGpuDraw', "the surface's passes and draws do not record: " + error.message);
        return plan;
      });
    });
    var done = function () { gpu.building--; };
    built.then(done, done);
    return built;
  }

  // The plan `given` makes on `device` from its pipelines and the copies
  // each pass and draw takes, `draws` and `passes`, in their order: each
  // array a buffer of the plan's own, the frame uniform made when a pass or
  // a draw reads it, the bind groups from group 0 to the last any binding
  // names, and every pass and draw recorded once into a pass of one-pixel
  // attachments of the plan's samples and depth, finished unsubmitted.
  function assemble(device, given, draws, passes, names) {
    var clears = clearsOf(given);
    var plan = {
      device: device, clear: given.clear, names: names, frame: null,
      samples: given.draws.length ? given.draws[0].spec.sampleCount : 1, depthTarget: given.draws.length ? given.draws[0].spec.depthTarget : null,
      clearDepth: clears.depth, clearStencil: clears.stencil,
      draws: [], passes: []
    };
    var reads = given.draws.some(function (d) { return d.frame; }) || given.passes.some(function (p) { return p.frame; });
    if (reads) {
      var bytes = new Uint8Array(Math.ceil((160 + given.uniform.length) / 16) * 16);
      bytes.set(new Uint8Array(header(0, 0, [0, 0], [-1, -1], darkTheme(), window.devicePixelRatio || 1, identity, identity).buffer));
      bytes.set(given.uniform, 160);
      plan.frame = device.createBuffer({ size: bytes.length, usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST });
      device.queue.writeBuffer(plan.frame, 0, bytes);
    }
    // What resource `r` binds, from its copy `c` or, for an array, a buffer
    // of the plan's own with its bytes.
    var bound = function (r, c) {
      if (r.bytes) {
        var buffer = device.createBuffer({ size: Math.max(4, Math.ceil(r.bytes.length / 4) * 4), usage: anyBufferUse(), mappedAtCreation: true });
        new Uint8Array(buffer.getMappedRange()).set(r.bytes);
        buffer.unmap();
        return { buffer: buffer };
      }
      if (r.buffer) return { buffer: c.object };
      if (r.texture) return { view: c.view };
      return { sampler: c.object };
    };
    // The bind groups `bindings` give `pipeline`, from group 0 to the last
    // they name, with the frame uniform at 1:0 when `frame` says the stages
    // read it.
    var groups = function (pipeline, bindings, taken, frame) {
      var entries = [];
      bindings.forEach(function (b, i) {
        var x = bound(b.resource, taken[i]);
        (entries[b.group] || (entries[b.group] = [])).push({ binding: b.binding, resource: x.buffer ? { buffer: x.buffer } : x.view || x.sampler });
      });
      if (frame) (entries[1] || (entries[1] = [])).push({ binding: 0, resource: { buffer: plan.frame } });
      var made = [];
      for (var g = 0; g < entries.length; g++) made.push(device.createBindGroup({ layout: pipeline.getBindGroupLayout(g), entries: entries[g] || [] }));
      return made;
    };
    given.passes.forEach(function (p, at) {
      var parts = passes[at];
      plan.passes.push({
        pipeline: parts[0],
        groups: groups(parts[0], p.bindings, parts[1], p.frame),
        workgroups: p.workgroups,
        indirect: p.indirect ? { buffer: parts[2][0].object, offset: p.indirect.offset } : null,
        once: p.once,
        ran: false
      });
    });
    given.draws.forEach(function (d, at) {
      var parts = draws[at];
      plan.draws.push({
        pipeline: parts[0],
        groups: groups(parts[0], d.bindings, parts[2], d.frame),
        vertices: d.vertices.map(function (v, i) { return bound(v.resource, parts[1][i]).buffer; }),
        index: d.index ? { buffer: bound(d.index.resource, parts[3][0]).buffer, format: d.index.format } : null,
        count: d.vertexCount,
        instances: d.instanceCount,
        blendConstant: d.blendConstant,
        stencilReference: d.stencilReference === null ? 0 : d.stencilReference
      });
    });
    var attachment = function (format, count) { return device.createTexture({ size: [1, 1], format: format, sampleCount: count, usage: GPUTextureUsage.RENDER_ATTACHMENT }).createView(); };
    var encoder = device.createCommandEncoder();
    if (plan.passes.length) {
      var computing = encoder.beginComputePass();
      plan.passes.forEach(function (p) { dispatch(computing, p); });
      computing.end();
    }
    var drawing = encoder.beginRenderPass({
      colorAttachments: [{ view: attachment('rgba8unorm', plan.samples), resolveTarget: plan.samples > 1 ? attachment('rgba8unorm', 1) : undefined, clearValue: plan.clear, loadOp: 'clear', storeOp: 'store' }],
      depthStencilAttachment: plan.depthTarget ? depthAttachment(plan, attachment(plan.depthTarget.format, plan.samples)) : undefined
    });
    plan.draws.forEach(function (d) { record(drawing, d); });
    drawing.end();
    encoder.finish();
    return plan;
  }

  // Records pass `p` of a plan into compute pass `pass`.
  function dispatch(pass, p) {
    pass.setPipeline(p.pipeline);
    p.groups.forEach(function (g, i) { pass.setBindGroup(i, g); });
    if (p.indirect) pass.dispatchWorkgroupsIndirect(p.indirect.buffer, p.indirect.offset);
    else pass.dispatchWorkgroups(p.workgroups[0], p.workgroups[1], p.workgroups[2]);
  }

  // Records draw `d` of a plan into render pass `pass`, with its own
  // stencil reference, 0 for a draw given none, whatever a draw before it
  // in the pass set.
  function record(pass, d) {
    pass.setPipeline(d.pipeline);
    d.groups.forEach(function (g, i) { pass.setBindGroup(i, g); });
    d.vertices.forEach(function (buffer, i) { pass.setVertexBuffer(i, buffer); });
    if (d.blendConstant) pass.setBlendConstant(d.blendConstant);
    pass.setStencilReference(d.stencilReference);
    if (d.index) {
      pass.setIndexBuffer(d.index.buffer, d.index.format);
      pass.drawIndexed(d.count, d.instances);
    } else {
      pass.draw(d.count, d.instances);
    }
  }

  // Builds `given` for surface `s` with the page's WebGPU device or its
  // WebGL 2 context once the surface's builds before it are done, and gives
  // it to the surface once every pass and draw is taken, replying to the
  // helper's request `ask` either way; given again with no request, as to a
  // page loaded again or on a new device, a build that fails is reported.
  // With `given` null, what the surface has when the build starts is built
  // again, as its WebGL 2 context comes back, unless a build since the
  // context last came back already made its plan.
  function give(s, given, ask) {
    var round = s.glRound, wanted = given;
    s.builds = s.builds.then(function () {
      if (!given) {
        wanted = s.given;
        if (!wanted || (s.plan && s.plan.round === round)) return null;
      }
      return gpuMode().then(function (mode) {
        if (mode === 'webgl2') return buildGl(s, wanted);
        return gpuDevice().then(function (device) { return build(device, wanted); });
      });
    }).then(function (plan) {
      if (!plan) return;
      var before = s.plan;
      s.given = wanted;
      s.plan = plan;
      if (before && before.gl && before !== plan) releaseGl(before);
      s.reported = null;
      s.dirty = true;
      sweepCopies();
      schedule();
      if (ask !== null) post({ t: 'gpu', request: ask, ok: null });
    }, function (err) {
      sweepCopies();
      if (ask !== null) post({ t: 'gpu', request: ask, error: { id: err.id || 'GuiGpuDraw', text: err.message } });
      else surfaceProblem(s, err.message);
    });
  }

  // The first 160 bytes of a surface's frame uniform, as WGSL lays out
  // `time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>,
  // dark: f32, scale: f32, view: mat4x4<f32>, projection: mat4x4<f32>`, at
  // bytes 0, 4, 8, 16, 24, 28, 32 and 96, `dark` 1 for the dark theme and 0
  // for the light, the matrices column by column.
  function header(time, delta, resolution, pointer, dark, scale, view, projection) {
    var f = new Float32Array(40);
    f[0] = time;
    f[1] = delta;
    f[2] = resolution[0];
    f[3] = resolution[1];
    f[4] = pointer[0];
    f[5] = pointer[1];
    f[6] = dark ? 1 : 0;
    f[7] = scale;
    f.set(view, 8);
    f.set(projection, 24);
    return f;
  }

  // Whether the window draws in its dark theme: the theme it was given,
  // or else the system's preference, as the stylesheet chooses its palette.
  function darkTheme() {
    var given = document.documentElement.dataset.theme;
    return given === 'dark' || (given !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
  }

  // Where surface `s` is drawn in the window's physical pixels: its left
  // and top edges and its size, each edge rounded as egui-wgpu rounds a
  // paint callback's viewport, and its pixels per point.
  function placeOf(s) {
    var r = s.canvas.getBoundingClientRect(), ppp = window.devicePixelRatio || 1;
    var left = Math.round(r.left * ppp), top = Math.round(r.top * ppp);
    return { left: left, top: top, across: Math.round(r.right * ppp) - left, down: Math.round(r.bottom * ppp) - top, ppp: ppp };
  }

  // Draws one frame of surface `s` into its canvas at its size in physical
  // pixels, as the egui renderer draws one: the frame uniform's header
  // written, the passes due run, then the draws over its clear color, into
  // multisampled and depth targets when its plan has them. With `read`, a
  // buffer and its bytes per row, the frame's texels are copied into it. A
  // page drawing with WebGL 2 draws the frame with `frameGl`, `read` taking
  // the texels. Returns where it drew, or null when the surface has no
  // pixels or the page no device.
  function frame(s, read) {
    if (gpu.mode === 'webgl2') return frameGl(s, read, null);
    var device = gpu.device;
    if (!device || !s.canvas) return null;
    if (!s.context) s.context = s.canvas.getContext('webgpu');
    if (!s.context) return null;
    var at = placeOf(s);
    if (at.across < 1 || at.down < 1) return null;
    var plan = s.plan && s.plan.device === device ? s.plan : null;
    if (s.canvas.width !== at.across) s.canvas.width = at.across;
    if (s.canvas.height !== at.down) s.canvas.height = at.down;
    if (s.configured !== device) {
      s.context.configure({ device: device, format: 'rgba8unorm', alphaMode: 'opaque', usage: GPUTextureUsage.RENDER_ATTACHMENT | GPUTextureUsage.COPY_SRC });
      s.configured = device;
    }
    var samples = plan ? plan.samples : 1, depth = plan ? depthFormatOf(plan.depthTarget) : null;
    var t = s.targets;
    if (!t || t.device !== device || t.across !== at.across || t.down !== at.down || t.samples !== samples || t.depth !== depth) {
      if (t) t.textures.forEach(function (texture) { texture.destroy(); });
      var textures = [];
      var make = function (format) {
        var texture = device.createTexture({ size: [at.across, at.down], format: format, sampleCount: samples, usage: GPUTextureUsage.RENDER_ATTACHMENT });
        textures.push(texture);
        return texture.createView();
      };
      t = s.targets = { device: device, across: at.across, down: at.down, samples: samples, depth: depth, color: samples > 1 ? make('rgba8unorm') : null, depthView: depth ? make(depth) : null, textures: textures };
    }
    var now = performance.now() / 1000;
    var delta = s.last === null ? 0 : now - s.last;
    s.last = now;
    s.dirty = false;
    device.pushErrorScope('validation');
    if (plan && plan.frame) {
      var pointer = s.hover ? [s.hover[0] * at.ppp - at.left, s.hover[1] * at.ppp - at.top] : [-1, -1];
      var view = s.camera ? cameraView(s.camera) : identity, projection = s.camera ? cameraProjection(s.camera, at.across / at.down) : identity;
      device.queue.writeBuffer(plan.frame, 0, header(now - s.started, delta, [at.across, at.down], pointer, darkTheme(), at.ppp, view, projection));
    }
    var encoder = device.createCommandEncoder();
    if (plan) {
      var due = plan.passes.filter(function (p) { return !p.once || !p.ran; });
      if (due.length) {
        var computing = encoder.beginComputePass();
        due.forEach(function (p) { p.ran = true; dispatch(computing, p); });
        computing.end();
      }
    }
    var texture = s.context.getCurrentTexture();
    var view = texture.createView();
    var drawing = encoder.beginRenderPass({
      colorAttachments: [{ view: t.color || view, resolveTarget: t.color ? view : undefined, clearValue: plan ? plan.clear : unset(), loadOp: 'clear', storeOp: 'store' }],
      depthStencilAttachment: t.depthView ? depthAttachment(plan, t.depthView) : undefined
    });
    if (plan) plan.draws.forEach(function (d) { record(drawing, d); });
    drawing.end();
    if (read) encoder.copyTextureToBuffer({ texture: texture }, { buffer: read.buffer, bytesPerRow: read.rowBytes }, [at.across, at.down]);
    device.queue.submit([encoder.finish()]);
    device.popErrorScope().then(function (error) { if (error) surfaceProblem(s, error.message); });
    return at;
  }

  // Whether surface `s` draws every frame: its plan reads the frame uniform
  // or has a pass run every frame.
  function animates(s) {
    return !!s.plan && (s.plan.frame !== null || s.plan.passes.some(function (p) { return !p.once; }));
  }

  // Requests an animation frame, in which each surface drawn in the window
  // reports the pointer's move since the last frame and its camera's, and
  // draws when it waits for a frame or draws every frame.
  function schedule() {
    if (framing) return;
    framing = true;
    requestAnimationFrame(function () {
      framing = false;
      var again = false;
      Object.keys(surfaces).forEach(function (id) {
        var s = surfaces[id];
        if (!s.canvas || !s.canvas.isConnected) return;
        if (s.moveDue) {
          s.moveDue = false;
          reportMove(s);
        }
        if (s.cameraMoved && s.camera) {
          s.cameraMoved = false;
          post({ t: 'camera', node: s.id, pose: { target: s.camera.target, distance: s.camera.distance, yaw: s.camera.yaw, pitch: s.camera.pitch } });
        }
        var every = animates(s);
        if (s.dirty || every) frame(s, null);
        again = again || every;
      });
      // A canvas takes a frame when the person did something on it, during
      // a drag of its view, or when it waits for a frame or was moving at
      // its last one, so the frame after the last move draws where the
      // move ends.
      var now = seconds();
      Object.keys(canvases).forEach(function (id) {
        var c = canvases[id];
        if (!c.canvas || !c.canvas.isConnected) return;
        if (c.due || c.dirty || c.animating || c.held) again = canvasPass(c, now) || again;
      });
      if (again) schedule();
    });
  }

  // Forgets the surfaces and canvases no longer in the window's tree, and
  // stops placing the column lines of tables no longer in it.
  function forgetGone() {
    Object.keys(tables).forEach(function (id) {
      if (find(tree, Number(id)) || !tables[id].edges) return;
      tables[id].edges.disconnect();
      tables[id].edges = null;
    });
    Object.keys(surfaces).forEach(function (id) {
      if (find(tree, Number(id))) return;
      if (surfaces[id].observer) surfaces[id].observer.disconnect();
      delete surfaces[id];
    });
    Object.keys(canvases).forEach(function (id) {
      var n = find(tree, Number(id));
      if (n && n.kind === 'Canvas') return;
      if (canvases[id].observer) canvases[id].observer.disconnect();
      clearTimeout(canvases[id].decideTimer);
      delete canvases[id];
    });
    sweepCopies();
  }

  // Whether node `id` lies inside a Scroll, which puts no limit on the
  // height of what it contains.
  function inScroll(id) {
    var walk = function (n, inside) {
      if (n.id === id) return inside;
      var kids = n.children || [];
      for (var i = 0; i < kids.length; i++) {
        var found = walk(kids[i], inside || n.kind === 'Scroll');
        if (found !== null) return found;
      }
      return null;
    };
    return walk(tree, false) === true;
  }

  // Bytes as base64.
  function base64Of(bytes) {
    var parts = [];
    for (var i = 0; i < bytes.length; i += 0x8000) parts.push(String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000)));
    return btoa(parts.join(''));
  }

  // WebGL 2's names for what a surface's JSON gives in WebGPU's: each
  // texture format's internal format, format and type, the typed array its
  // texels upload as, and whether its bytes come blue first, which WebGL 2
  // has no format for, so they upload red first; each blend factor and
  // operation; each topology; each comparison and stencil operation; and
  // each depth format's renderbuffer format and the attachment it takes.
  var glTextureFormats = {
    'r8unorm': ['R8', 'RED', 'UNSIGNED_BYTE', Uint8Array, false],
    'rg8unorm': ['RG8', 'RG', 'UNSIGNED_BYTE', Uint8Array, false],
    'rgba8unorm': ['RGBA8', 'RGBA', 'UNSIGNED_BYTE', Uint8Array, false],
    'rgba8unorm-srgb': ['SRGB8_ALPHA8', 'RGBA', 'UNSIGNED_BYTE', Uint8Array, false],
    'bgra8unorm': ['RGBA8', 'RGBA', 'UNSIGNED_BYTE', Uint8Array, true],
    'bgra8unorm-srgb': ['SRGB8_ALPHA8', 'RGBA', 'UNSIGNED_BYTE', Uint8Array, true],
    'r16float': ['R16F', 'RED', 'HALF_FLOAT', Uint16Array, false],
    'rg16float': ['RG16F', 'RG', 'HALF_FLOAT', Uint16Array, false],
    'rgba16float': ['RGBA16F', 'RGBA', 'HALF_FLOAT', Uint16Array, false],
    'r32float': ['R32F', 'RED', 'FLOAT', Float32Array, false],
    'rg32float': ['RG32F', 'RG', 'FLOAT', Float32Array, false],
    'rgba32float': ['RGBA32F', 'RGBA', 'FLOAT', Float32Array, false],
    'r32uint': ['R32UI', 'RED_INTEGER', 'UNSIGNED_INT', Uint32Array, false],
    'r32sint': ['R32I', 'RED_INTEGER', 'INT', Int32Array, false]
  };
  var glFactors = {
    'zero': 'ZERO', 'one': 'ONE', 'src': 'SRC_COLOR', 'one-minus-src': 'ONE_MINUS_SRC_COLOR', 'src-alpha': 'SRC_ALPHA', 'one-minus-src-alpha': 'ONE_MINUS_SRC_ALPHA',
    'dst': 'DST_COLOR', 'one-minus-dst': 'ONE_MINUS_DST_COLOR', 'dst-alpha': 'DST_ALPHA', 'one-minus-dst-alpha': 'ONE_MINUS_DST_ALPHA',
    'src-alpha-saturated': 'SRC_ALPHA_SATURATE', 'constant': 'CONSTANT_COLOR', 'one-minus-constant': 'ONE_MINUS_CONSTANT_COLOR'
  };
  var glOperations = { 'add': 'FUNC_ADD', 'subtract': 'FUNC_SUBTRACT', 'reverse-subtract': 'FUNC_REVERSE_SUBTRACT', 'min': 'MIN', 'max': 'MAX' };
  var glTopologies = { 'point-list': 'POINTS', 'line-list': 'LINES', 'line-strip': 'LINE_STRIP', 'triangle-list': 'TRIANGLES', 'triangle-strip': 'TRIANGLE_STRIP' };
  var glCompares = { 'never': 'NEVER', 'less': 'LESS', 'equal': 'EQUAL', 'less-equal': 'LEQUAL', 'greater': 'GREATER', 'not-equal': 'NOTEQUAL', 'greater-equal': 'GEQUAL', 'always': 'ALWAYS' };
  var glStencilOperations = {
    'keep': 'KEEP', 'zero': 'ZERO', 'replace': 'REPLACE', 'invert': 'INVERT',
    'increment-clamp': 'INCR', 'decrement-clamp': 'DECR', 'increment-wrap': 'INCR_WRAP', 'decrement-wrap': 'DECR_WRAP'
  };
  var glDepthFormats = {
    'stencil8': ['STENCIL_INDEX8', 'STENCIL_ATTACHMENT'],
    'depth16unorm': ['DEPTH_COMPONENT16', 'DEPTH_ATTACHMENT'],
    'depth24plus': ['DEPTH_COMPONENT24', 'DEPTH_ATTACHMENT'],
    'depth24plus-stencil8': ['DEPTH24_STENCIL8', 'DEPTH_STENCIL_ATTACHMENT'],
    'depth32float': ['DEPTH_COMPONENT32F', 'DEPTH_ATTACHMENT'],
    'depth32float-stencil8': ['DEPTH32F_STENCIL8', 'DEPTH_STENCIL_ATTACHMENT']
  };

  // What the helper writes each draw's WGSL as for WebGL 2, by the shader
  // and its entry points and constants, and the requests it has yet to
  // reply to, by their number.
  var glslWritten = {};
  var glslAsks = {};
  var glslNext = 0;
  // The draw `spec` as GLSL ES 3.00, requested from the helper once: each
  // stage's GLSL with the names of its uniform blocks and samplers by the
  // group and binding they stand for, or why WebGL 2 cannot draw it.
  function glslOf(spec) {
    var key = JSON.stringify([spec.source, spec.vertexEntry, spec.fragmentEntry, spec.constants]);
    if (glslWritten[key]) return glslWritten[key];
    var ask = ++glslNext;
    glslWritten[key] = new Promise(function (resolve, reject) {
      glslAsks[ask] = { resolve: resolve, reject: reject };
      post({ t: 'glsl', request: ask, source: spec.source, vertexEntry: spec.vertexEntry, fragmentEntry: spec.fragmentEntry, constants: spec.constants });
    }).then(null, function (err) {
      // A request the helper refuses is sent again by the next build that
      // wants it.
      delete glslWritten[key];
      throw err;
    });
    return glslWritten[key];
  }

  // Surface `s`'s WebGL 2 context, made on its canvas the first time, its
  // float textures filtered where the engine can filter them.
  function glOf(s) {
    if (s.gl) return s.gl;
    var gl = canvasOf(s).getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false, premultipliedAlpha: false, preserveDrawingBuffer: true });
    if (!gl) throw gpuError('GuiGpuNoWebGpu', engineName + ' gives the surface no WebGL 2 context, so it draws no GpuSurface; a window opened with -Renderer Egui draws it');
    gl.getExtension('OES_texture_float_linear');
    s.gl = gl;
    s.glCopies = {};
    s.glTargets = null;
    return gl;
  }

  // Takes the copy resource `r` names as a page drawing with WebGL 2 stores
  // it: a buffer's or texture's bytes fetched from the helper at its
  // version or a later one, which each surface's context uploads, or a
  // sampler's settings.
  function takeBytes(r) {
    var name = r.buffer || r.texture || r.sampler;
    var c = copies[name] || (copies[name] = { version: 0, resource: r, device: null, object: null, view: null, bytes: null, takes: Promise.resolve() });
    var taken = c.takes.then(function () {
      if (c.version >= r.version && (r.sampler || c.bytes)) return c;
      return (r.sampler ? Promise.resolve(null) : copyBytes(name, r.version)).then(function (bytes) {
        if (r.buffer && bytes.byteLength !== r.length) throw gpuError('GuiGpuNoCopy', name + ' came with ' + bytes.byteLength + ' bytes for a buffer of ' + r.length + ' bytes');
        if (r.texture && bytes.byteLength !== r.width * r.height * r.texelBytes) throw gpuError('GuiGpuNoCopy', name + ' came with ' + bytes.byteLength + ' bytes for its ' + r.width + ' by ' + r.height + ' texels of ' + r.format);
        c.bytes = bytes;
        c.version = r.version;
        c.resource = r;
        return c;
      });
    });
    // The next take of the copy waits for this one however it went; this
    // one's caller gets the outcome.
    c.takes = taken.then(function () {}, function () {});
    return taken;
  }

  // Uploads the bytes or settings of copy `c` into `object`, its form
  // `form` in context `gl`: a buffer to draw or bind, an index buffer, which
  // WebGL 2 cannot use as any other kind of buffer, a texture or a sampler.
  function glUpload(gl, form, object, c) {
    var r = c.resource;
    if (form === 'sampler') {
      var filter = r.filter === 'linear' ? gl.LINEAR : gl.NEAREST;
      var wrap = { 'clamp-to-edge': gl.CLAMP_TO_EDGE, 'repeat': gl.REPEAT, 'mirror-repeat': gl.MIRRORED_REPEAT }[r.address];
      gl.samplerParameteri(object, gl.MIN_FILTER, filter);
      gl.samplerParameteri(object, gl.MAG_FILTER, filter);
      gl.samplerParameteri(object, gl.TEXTURE_WRAP_S, wrap);
      gl.samplerParameteri(object, gl.TEXTURE_WRAP_T, wrap);
      gl.samplerParameteri(object, gl.TEXTURE_WRAP_R, wrap);
    } else if (form === 'texture') {
      var f = glTextureFormats[r.format];
      if (!f) throw gpuError('GuiGpuNoWebGpu', engineName + ' has no WebGPU, and WebGL 2 takes no texture of ' + r.format + '; a window opened with -Renderer Egui draws it');
      var texels = new Uint8Array(c.bytes.slice(0));
      if (f[4]) for (var i = 0; i + 3 < texels.length; i += 4) { var blue = texels[i]; texels[i] = texels[i + 2]; texels[i + 2] = blue; }
      gl.bindTexture(gl.TEXTURE_2D, object);
      gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl[f[0]], r.width, r.height, 0, gl[f[1]], gl[f[2]], new f[3](texels.buffer));
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 0);
    } else {
      var target = form === 'index' ? gl.ELEMENT_ARRAY_BUFFER : gl.ARRAY_BUFFER;
      var padded = new Uint8Array(Math.max(4, Math.ceil(c.bytes.byteLength / 4) * 4));
      padded.set(new Uint8Array(c.bytes));
      // An index buffer bound here must not land in a vertex array's state.
      gl.bindVertexArray(null);
      gl.bindBuffer(target, object);
      gl.bufferData(target, padded, gl.DYNAMIC_DRAW);
    }
  }

  // Copy `c`'s form `form` in surface `s`'s context, uploaded again when
  // the copy is newer than what the context has; the same object, so the
  // plans with it read the new bytes.
  function glCopy(s, c, form) {
    var gl = s.gl;
    var name = c.resource.buffer || c.resource.texture || c.resource.sampler;
    var held = s.glCopies[name] || (s.glCopies[name] = {});
    var f = held[form];
    if (f && f.version === c.version) return f.object;
    var object = f ? f.object : form === 'texture' ? gl.createTexture() : form === 'sampler' ? gl.createSampler() : gl.createBuffer();
    glUpload(gl, form, object, c);
    held[form] = { object: object, version: c.version };
    return object;
  }

  // Surface `s`'s context uploads copy `c` again into each form it has.
  function glRefresh(s, c) {
    var name = c.resource.buffer || c.resource.texture || c.resource.sampler;
    Object.keys((s.glCopies || {})[name] || {}).forEach(function (form) { glCopy(s, c, form); });
  }

  // A buffer of the plan's own in context `gl` containing array `bytes`, an
  // index buffer when `index`.
  function glArray(gl, bytes, index) {
    var buffer = gl.createBuffer();
    var padded = new Uint8Array(Math.max(4, Math.ceil(bytes.length / 4) * 4));
    padded.set(bytes);
    var target = index ? gl.ELEMENT_ARRAY_BUFFER : gl.ARRAY_BUFFER;
    gl.bindVertexArray(null);
    gl.bindBuffer(target, buffer);
    gl.bufferData(target, padded, gl.STATIC_DRAW);
    return buffer;
  }

  // The program of a draw's GLSL `written` in context `gl`, compiled and
  // linked, or why WebGL 2 refused it.
  function glProgram(gl, written, at) {
    var stage = function (type, text, what) {
      var shader = gl.createShader(type);
      gl.shaderSource(shader, text);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw gpuError('GuiGpuDraw', 'WebGL 2 does not compile draw ' + at + "'s " + what + ' stage: ' + gl.getShaderInfoLog(shader));
      return shader;
    };
    var program = gl.createProgram();
    var shaders = [stage(gl.VERTEX_SHADER, written.vertex.glsl, 'vertex'), stage(gl.FRAGMENT_SHADER, written.fragment.glsl, 'fragment')];
    shaders.forEach(function (shader) { gl.attachShader(program, shader); });
    gl.linkProgram(program);
    // The linked program has all it needs of its stages, which go with it.
    shaders.forEach(function (shader) { gl.deleteShader(shader); });
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw gpuError('GuiGpuDraw', 'WebGL 2 does not link draw ' + at + "'s stages: " + gl.getProgramInfoLog(program));
    return program;
  }

  // Lets go of what a WebGL 2 plan made for itself: its programs, vertex
  // arrays and the buffers with its arrays and frame uniform.
  function releaseGl(plan) {
    var gl = plan.gl;
    if (gl.isContextLost()) return;
    plan.draws.forEach(function (d) {
      gl.deleteProgram(d.program);
      gl.deleteVertexArray(d.vao);
    });
    plan.owned.forEach(function (buffer) { gl.deleteBuffer(buffer); });
  }

  // A vertex format's numbers per vertex, its WebGL 2 type, and whether
  // the shader reads it as integers.
  function glVertexFormat(gl, format) {
    var m = /^(float32|float16|sint32|uint32)(?:x([234]))?$/.exec(format);
    if (!m) throw gpuError('GuiGpuDraw', 'WebGL 2 reads no vertex format ' + format);
    var type = { float32: gl.FLOAT, float16: gl.HALF_FLOAT, sint32: gl.INT, uint32: gl.UNSIGNED_INT }[m[1]];
    return { size: m[2] ? Number(m[2]) : 1, type: type, integer: m[1] === 'sint32' || m[1] === 'uint32' };
  }

  // `given` built in surface `s`'s WebGL 2 context, as a page whose engine
  // offers no WebGPU draws it: each draw's program from the GLSL the helper
  // writes, its vertex array, its uniform blocks and samplers bound to the
  // copies and arrays it reads, and the frame uniform when one reads it;
  // then a frame drawn once, unshown, so what WebGL 2 refuses is refused
  // before the surface takes it. A compute pass, a storage buffer or
  // texture, dual-source blending, and a clamped depth bias on an engine
  // whose WebGL 2 lacks EXT_polygon_offset_clamp are refused as needing
  // WebGPU. A context the engine has taken away is waited for, and a build
  // it is taken from partway starts again once it comes back. Returns the
  // plan.
  function buildGl(s, given) {
    var needs = function (what) { return Promise.reject(gpuError('GuiGpuNoWebGpu', engineName + ' has no WebGPU, and ' + what + '; a window opened with -Renderer Egui draws it')); };
    if (given.passes.length) return needs('the surface has a compute pass, which WebGL 2 has none of');
    var stopped = drawsRefused(given);
    if (stopped) return Promise.reject(stopped);
    for (var d = 0; d < given.draws.length; d++) {
      var blend = given.draws[d].spec.blend;
      if (blend && [blend.color.srcFactor, blend.color.dstFactor, blend.alpha.srcFactor, blend.alpha.dstFactor].some(function (f) { return !glFactors[f]; })) {
        return needs('draw ' + d + ' blends with a second source, which WebGL 2 has none of');
      }
    }
    var gl;
    try {
      gl = glOf(s);
    } catch (err) {
      return Promise.reject(err);
    }
    var again = function () { return glReturned(s).then(function () { return buildGl(s, given); }); };
    if (gl.isContextLost()) return again();
    var clamp = gl.getExtension('EXT_polygon_offset_clamp');
    for (var c = 0; c < given.draws.length; c++) {
      var state = given.draws[c].spec.depthStencil;
      if (state && state.depthBiasClamp !== 0 && !clamp) return needs('draw ' + c + "'s pipeline clamps its depth bias, which WebGL 2 does only through the EXT_polygon_offset_clamp extension, and the engine offers none");
    }
    var names = [];
    var copyOf = function (r) {
      if (r.missing) return Promise.reject(gpuError('GuiGpuNoCopy', 'the HTML renderer has no copy of ' + r.missing));
      if (r.bytes) return Promise.resolve(null);
      var name = r.buffer || r.texture || r.sampler;
      if (names.indexOf(name) < 0) names.push(name);
      return takeBytes(r);
    };
    var copiesOf = function (resources) { return Promise.all(resources.map(copyOf)); };
    gpu.building++;
    var built = Promise.all(given.draws.map(function (draw) {
      return Promise.all([glslOf(draw.spec), copiesOf(draw.vertices.map(function (v) { return v.resource; })), copiesOf(draw.bindings.map(function (b) { return b.resource; })), copiesOf(draw.index ? [draw.index.resource] : [])]);
    })).then(function (parts) {
      var plan = null, error = gl.NO_ERROR;
      // A context taken away partway makes its calls fail or come back
      // empty, so what fails then is the loss and not the draws.
      try {
        plan = assembleGl(s, gl, given, parts, names, clamp);
        gl.getError();
        frameGl(s, null, plan);
        error = gl.getError();
      } catch (err) {
        if (!gl.isContextLost()) throw err;
      }
      if (gl.isContextLost()) return null;
      if (error !== gl.NO_ERROR) {
        releaseGl(plan);
        throw gpuError('GuiGpuDraw', "the surface's draws do not draw in WebGL 2: error 0x" + error.toString(16));
      }
      plan.round = s.glRound;
      return plan;
    });
    var done = function () { gpu.building--; };
    built.then(done, done);
    return built.then(function (plan) { return plan || again(); });
  }

  // The return of surface `s`'s WebGL 2 context, made when the engine takes
  // it away and stored until it comes back.
  function glBackOf(s) {
    if (!s.glBack) {
      var back = { done: null };
      back.promise = new Promise(function (resolve) { back.done = resolve; });
      s.glBack = back;
    }
    return s.glBack;
  }

  // Waits for surface `s`'s WebGL 2 context to come back, refused when the
  // engine has not given it back within the 30 seconds the helper waits for
  // a page's capture.
  function glReturned(s) {
    if (!s.gl.isContextLost()) return Promise.resolve();
    var back = glBackOf(s);
    return new Promise(function (resolve, reject) {
      var late = window.setTimeout(function () {
        reject(gpuError('GuiGpuDraw', "the engine took the surface's WebGL 2 context away and has not given it back within 30 seconds"));
      }, 30000);
      back.promise.then(function () {
        window.clearTimeout(late);
        resolve();
      });
    });
  }

  // The plan `given` makes in context `gl` from each draw's GLSL and the
  // copies it takes, `parts`, in draw order: each draw's program, vertex
  // array, uniform blocks with the binding points they take and the
  // buffers bound there, samplers with the texture units they take and the
  // textures and samplers bound there, its blend, cull, depth test,
  // stencil test with its reference, depth bias, drawn through `clamp`,
  // the EXT_polygon_offset_clamp extension, when clamped, and counts; the
  // depth attachment and what it is cleared to; and the frame uniform when
  // one reads it.
  function assembleGl(s, gl, given, parts, names, clamp) {
    var clears = clearsOf(given);
    var plan = {
      gl: gl, clear: given.clear, names: names, frame: null, passes: [], draws: [], owned: [],
      samples: given.draws.length ? given.draws[0].spec.sampleCount : 1, depthTarget: given.draws.length ? given.draws[0].spec.depthTarget : null,
      clearDepth: clears.depth, clearStencil: clears.stencil
    };
    if (given.draws.some(function (d) { return d.frame; })) {
      var bytes = new Uint8Array(Math.ceil((160 + given.uniform.length) / 16) * 16);
      bytes.set(new Uint8Array(header(0, 0, [0, 0], [-1, -1], darkTheme(), window.devicePixelRatio || 1, identity, identity).buffer));
      bytes.set(given.uniform, 160);
      plan.frame = glArray(gl, bytes, false);
      plan.owned.push(plan.frame);
    }
    given.draws.forEach(function (d, at) {
      var written = parts[at][0];
      var program = glProgram(gl, written, at);
      // The buffer, texture or sampler binding `group`:`binding` reads.
      var bound = function (group, binding, form) {
        if (group === 1 && binding === 0 && d.frame) return plan.frame;
        var i = d.bindings.findIndex(function (b) { return b.group === group && b.binding === binding; });
        if (i < 0) return null;
        var r = d.bindings[i].resource;
        if (r.bytes) {
          var own = glArray(gl, new Uint8Array(r.bytes), false);
          plan.owned.push(own);
          return own;
        }
        return glCopy(s, parts[at][2][i], form);
      };
      var points = {}, blocks = [];
      var textures = [];
      gl.useProgram(program);
      [written.vertex, written.fragment].forEach(function (stage) {
        stage.blocks.forEach(function (b) {
          var key = b.at[0] + ':' + b.at[1];
          if (!(key in points)) {
            points[key] = blocks.length;
            blocks.push({ point: blocks.length, buffer: bound(b.at[0], b.at[1], 'buffer') });
          }
          var index = gl.getUniformBlockIndex(program, b.name);
          if (index !== gl.INVALID_INDEX) gl.uniformBlockBinding(program, index, points[key]);
        });
        stage.samplers.forEach(function (sm) {
          var location = gl.getUniformLocation(program, sm.name);
          if (!location || !sm.texture) return;
          var unit = textures.length;
          textures.push({ unit: unit, texture: bound(sm.texture[0], sm.texture[1], 'texture'), sampler: sm.sampler ? bound(sm.sampler[0], sm.sampler[1], 'sampler') : null });
          gl.uniform1i(location, unit);
        });
      });
      var index = null;
      if (d.index) {
        var ir = d.index.resource;
        var indexBuffer = ir.bytes ? glArray(gl, new Uint8Array(ir.bytes), true) : glCopy(s, parts[at][3][0], 'index');
        if (ir.bytes) plan.owned.push(indexBuffer);
        index = { buffer: indexBuffer, type: d.index.format === 'uint16' ? gl.UNSIGNED_SHORT : gl.UNSIGNED_INT };
      }
      var vertexBuffers = d.vertices.map(function (v, i) {
        if (!v.resource.bytes) return glCopy(s, parts[at][1][i], 'buffer');
        var own = glArray(gl, new Uint8Array(v.resource.bytes), false);
        plan.owned.push(own);
        return own;
      });
      var vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      d.vertices.forEach(function (v, i) {
        var f = glVertexFormat(gl, v.format);
        gl.bindBuffer(gl.ARRAY_BUFFER, vertexBuffers[i]);
        gl.enableVertexAttribArray(v.location);
        if (f.integer) gl.vertexAttribIPointer(v.location, f.size, f.type, v.stride, 0);
        else gl.vertexAttribPointer(v.location, f.size, f.type, false, v.stride, 0);
      });
      if (index) gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, index.buffer);
      gl.bindVertexArray(null);
      var spec = d.spec, blend = spec.blend, target = spec.depthTarget, state = spec.depthStencil;
      var face = function (f) { return { compare: gl[glCompares[f.compare]], fail: gl[glStencilOperations[f.failOp]], depthFail: gl[glStencilOperations[f.depthFailOp]], pass: gl[glStencilOperations[f.passOp]] }; };
      var biased = state && (state.depthBias !== 0 || state.depthBiasSlopeScale !== 0 || state.depthBiasClamp !== 0);
      plan.draws.push({
        program: program,
        vao: vao,
        blocks: blocks,
        textures: textures,
        blend: blend ? {
          color: [gl[glFactors[blend.color.srcFactor]], gl[glFactors[blend.color.dstFactor]], gl[glOperations[blend.color.operation]]],
          alpha: [gl[glFactors[blend.alpha.srcFactor]], gl[glFactors[blend.alpha.dstFactor]], gl[glOperations[blend.alpha.operation]]]
        } : null,
        blendConstant: d.blendConstant,
        cull: spec.cull === 'front' ? gl.FRONT : spec.cull === 'back' ? gl.BACK : null,
        depth: target && target.depth ? { compare: gl[glCompares[state.depthCompare]], write: state.depthWriteEnabled } : null,
        stencil: target && target.stencil ? { front: face(state.stencilFront), back: face(state.stencilBack), read: state.stencilReadMask, write: state.stencilWriteMask } : null,
        stencilReference: d.stencilReference === null ? 0 : d.stencilReference,
        bias: biased ? { factor: state.depthBiasSlopeScale, units: state.depthBias, clamp: state.depthBiasClamp, extension: clamp } : null,
        mode: gl[glTopologies[spec.topology]],
        index: index,
        count: d.vertexCount,
        instances: d.instanceCount
      });
    });
    gl.useProgram(null);
    return plan;
  }

  // Draws draw `d` of a WebGL 2 plan with what it binds, its blend, cull,
  // depth test, stencil test and depth bias; a counterclockwise triangle
  // faces the viewer, which in the flipped space the GLSL draws in is a
  // clockwise one, so the front face's stencil test is WebGL 2's FRONT.
  function recordGl(gl, d) {
    gl.useProgram(d.program);
    gl.bindVertexArray(d.vao);
    d.blocks.forEach(function (b) { gl.bindBufferBase(gl.UNIFORM_BUFFER, b.point, b.buffer); });
    d.textures.forEach(function (t) {
      gl.activeTexture(gl.TEXTURE0 + t.unit);
      gl.bindTexture(gl.TEXTURE_2D, t.texture);
      gl.bindSampler(t.unit, t.sampler);
    });
    if (d.blend) {
      gl.enable(gl.BLEND);
      gl.blendFuncSeparate(d.blend.color[0], d.blend.color[1], d.blend.alpha[0], d.blend.alpha[1]);
      gl.blendEquationSeparate(d.blend.color[2], d.blend.alpha[2]);
    } else {
      gl.disable(gl.BLEND);
    }
    if (d.blendConstant) gl.blendColor(d.blendConstant[0], d.blendConstant[1], d.blendConstant[2], d.blendConstant[3]);
    if (d.cull) {
      gl.enable(gl.CULL_FACE);
      gl.cullFace(d.cull);
    } else {
      gl.disable(gl.CULL_FACE);
    }
    if (d.depth) {
      gl.enable(gl.DEPTH_TEST);
      gl.depthFunc(d.depth.compare);
      gl.depthMask(d.depth.write);
    } else {
      gl.disable(gl.DEPTH_TEST);
    }
    if (d.stencil) {
      gl.enable(gl.STENCIL_TEST);
      [[gl.FRONT, d.stencil.front], [gl.BACK, d.stencil.back]].forEach(function (f) {
        gl.stencilFuncSeparate(f[0], f[1].compare, d.stencilReference, d.stencil.read);
        gl.stencilOpSeparate(f[0], f[1].fail, f[1].depthFail, f[1].pass);
        gl.stencilMaskSeparate(f[0], d.stencil.write);
      });
    } else {
      gl.disable(gl.STENCIL_TEST);
    }
    if (d.bias) {
      gl.enable(gl.POLYGON_OFFSET_FILL);
      if (d.bias.clamp !== 0) d.bias.extension.polygonOffsetClampEXT(d.bias.factor, d.bias.units, d.bias.clamp);
      else gl.polygonOffset(d.bias.factor, d.bias.units);
    } else {
      gl.disable(gl.POLYGON_OFFSET_FILL);
    }
    if (d.index) gl.drawElementsInstanced(d.mode, d.count, d.index.type, 0, d.instances);
    else gl.drawArraysInstanced(d.mode, 0, d.count, d.instances);
  }

  // What surface `s` draws into in context `gl` at `across` by `down`
  // pixels: a color texture each frame ends in, drawn into through a
  // multisampled renderbuffer and resolved when the plan takes samples, and
  // a renderbuffer of `depth`, the plan's depth format as WebGPU names it,
  // when it has one; made again at another size or format.
  function glTargets(s, gl, across, down, samples, depth) {
    var t = s.glTargets;
    if (t && t.across === across && t.down === down && t.samples === samples && t.depth === depth) return t;
    if (t) {
      gl.deleteTexture(t.color);
      [t.multi, t.depthBuffer].forEach(function (r) { if (r) gl.deleteRenderbuffer(r); });
      [t.resolved, t.draw].forEach(function (f) { gl.deleteFramebuffer(f); });
    }
    var color = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, color);
    gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, across, down);
    var resolved = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, resolved);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0);
    var draw = resolved, multi = null, depthBuffer = null;
    if (samples > 1) {
      multi = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, multi);
      gl.renderbufferStorageMultisample(gl.RENDERBUFFER, samples, gl.RGBA8, across, down);
      draw = gl.createFramebuffer();
      gl.bindFramebuffer(gl.FRAMEBUFFER, draw);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, multi);
    }
    if (depth) {
      var held = glDepthFormats[depth];
      depthBuffer = gl.createRenderbuffer();
      gl.bindRenderbuffer(gl.RENDERBUFFER, depthBuffer);
      if (samples > 1) gl.renderbufferStorageMultisample(gl.RENDERBUFFER, samples, gl[held[0]], across, down);
      else gl.renderbufferStorage(gl.RENDERBUFFER, gl[held[0]], across, down);
      gl.bindFramebuffer(gl.FRAMEBUFFER, draw);
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl[held[1]], gl.RENDERBUFFER, depthBuffer);
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    s.glTargets = { across: across, down: down, samples: samples, depth: depth, color: color, resolved: resolved, draw: draw, multi: multi, depthBuffer: depthBuffer };
    return s.glTargets;
  }

  // Draws one frame of surface `s` with WebGL 2, as `frame` draws one with
  // WebGPU: into its targets, the multisampled ones resolved, then shown
  // flipped on its canvas, since the GLSL draws upside down. With `read`,
  // the frame's texels go into `read.pixels`, rows of red, green, blue and
  // alpha bytes from the top. With `trial`, a plan not yet taken is drawn
  // in its place and not shown, so a build finds out what WebGL 2 refuses.
  // Returns where it drew, or null when the surface has no pixels or no
  // context.
  function frameGl(s, read, trial) {
    var gl = s.gl;
    if (!gl || gl.isContextLost()) return null;
    var at = placeOf(s);
    if (at.across < 1 || at.down < 1) return null;
    var plan = trial || (s.plan && s.plan.gl === gl ? s.plan : null);
    if (s.canvas.width !== at.across) s.canvas.width = at.across;
    if (s.canvas.height !== at.down) s.canvas.height = at.down;
    var target = plan ? plan.depthTarget : null;
    var t = glTargets(s, gl, at.across, at.down, plan ? plan.samples : 1, depthFormatOf(target));
    var now = performance.now() / 1000;
    var delta = s.last === null ? 0 : now - s.last;
    if (!trial) {
      s.last = now;
      s.dirty = false;
    }
    if (plan && plan.frame) {
      var pointer = s.hover ? [s.hover[0] * at.ppp - at.left, s.hover[1] * at.ppp - at.top] : [-1, -1];
      var view = s.camera ? cameraView(s.camera) : identity, projection = s.camera ? cameraProjection(s.camera, at.across / at.down) : identity;
      gl.bindBuffer(gl.UNIFORM_BUFFER, plan.frame);
      gl.bufferSubData(gl.UNIFORM_BUFFER, 0, header(now - s.started, delta, [at.across, at.down], pointer, darkTheme(), at.ppp, view, projection));
    }
    var clear = plan ? plan.clear : unset();
    gl.bindFramebuffer(gl.FRAMEBUFFER, t.draw);
    gl.viewport(0, 0, at.across, at.down);
    gl.clearColor(clear[0], clear[1], clear[2], clear[3]);
    gl.clearDepth(plan ? plan.clearDepth : 1);
    gl.clearStencil(plan ? plan.clearStencil : 0);
    gl.depthMask(true);
    gl.stencilMask(0xffffffff);
    gl.clear(gl.COLOR_BUFFER_BIT | (target && target.depth ? gl.DEPTH_BUFFER_BIT : 0) | (target && target.stencil ? gl.STENCIL_BUFFER_BIT : 0));
    gl.frontFace(gl.CW);
    gl.blendColor(0, 0, 0, 0);
    if (plan) plan.draws.forEach(function (d) { recordGl(gl, d); });
    gl.bindVertexArray(null);
    if (t.multi) {
      gl.bindFramebuffer(gl.READ_FRAMEBUFFER, t.draw);
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, t.resolved);
      gl.blitFramebuffer(0, 0, at.across, at.down, 0, 0, at.across, at.down, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    }
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, t.resolved);
    if (!trial) {
      gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null);
      gl.blitFramebuffer(0, 0, at.across, at.down, 0, at.down, at.across, 0, gl.COLOR_BUFFER_BIT, gl.NEAREST);
    }
    if (read) {
      read.pixels = new Uint8Array(at.across * at.down * 4);
      gl.readPixels(0, 0, at.across, at.down, gl.RGBA, gl.UNSIGNED_BYTE, read.pixels);
    }
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    return at;
  }

  // A surface's camera, as proto's camera module moves it and draws
  // through it: angles in degrees, right-handed with +Y up, depth from 0 at
  // near to 1 at far.
  function radians(degrees) { return degrees * (Math.PI / 180); }
  function cross3(a, b) { return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]; }
  function dot3(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
  // The camera's right, up and backward unit vectors; right comes from the
  // yaw alone, so they stay a basis looking straight up or down.
  function cameraAxes(c) {
    var yaw = radians(c.yaw), pitch = radians(c.pitch);
    var back = [Math.cos(pitch) * Math.sin(yaw), Math.sin(pitch), Math.cos(pitch) * Math.cos(yaw)];
    var right = [Math.cos(yaw), 0, -Math.sin(yaw)];
    return { right: right, up: cross3(back, right), back: back };
  }
  // The matrix from the scene's coordinates to the camera's, column by
  // column.
  function cameraView(c) {
    var a = cameraAxes(c);
    var eye = [0, 1, 2].map(function (i) { return c.target[i] + c.distance * a.back[i]; });
    return [
      a.right[0], a.up[0], a.back[0], 0,
      a.right[1], a.up[1], a.back[1], 0,
      a.right[2], a.up[2], a.back[2], 0,
      -dot3(a.right, eye), -dot3(a.up, eye), -dot3(a.back, eye), 1
    ];
  }
  // The perspective matrix for a surface `aspect` times as wide as it is
  // tall, column by column.
  function cameraProjection(c, aspect) {
    var height = 1 / Math.tan(radians(c.fieldOfView) / 2);
    var depth = c.far / (c.near - c.far);
    return [height / aspect, 0, 0, 0, 0, height, 0, 0, 0, 0, depth, -1, 0, 0, depth * c.near, 0];
  }
  // Makes move `made` for a drag `across` and `down` points on a surface
  // `height` points tall: an orbit turns a full circle for a drag the
  // surface's height across, the yaw staying within -180 to 180 degrees
  // and the pitch within -90 to 90; a pan moves a point at the target's
  // depth with the pointer; a zoom drag moves 1 notch per 100 points up.
  function cameraDrag(c, made, across, down, height) {
    if (made === 'Orbit') {
      var yaw = (c.yaw - 360 * across / height + 180) % 360;
      if (yaw < 0) yaw += 360;
      c.yaw = yaw - 180;
      c.pitch = Math.min(Math.max(c.pitch + 360 * down / height, -90), 90);
    } else if (made === 'Pan') {
      var a = cameraAxes(c);
      var half = c.distance * Math.tan(radians(c.fieldOfView) / 2);
      var sideways = -2 * across * half / height, upward = 2 * down * half / height;
      for (var i = 0; i < 3; i++) c.target[i] += a.right[i] * sideways + a.up[i] * upward;
    } else {
      cameraZoom(c, -down / 100);
    }
  }
  // Moves the camera toward the target by `notches` wheel notches, each
  // scaling the distance by 0.95, as three.js OrbitControls zooms; away
  // for a negative count.
  function cameraZoom(c, notches) { c.distance *= Math.pow(0.95, notches); }
  // The move the camera's controls bind to gesture `g`, a drag of a button
  // or a wheel turn with modifier keys down, matched exactly, by the first
  // binding that matches; null for none.
  function cameraMoveFor(c, g) {
    for (var i = 0; i < c.bindings.length; i++) {
      var b = c.bindings[i];
      if (b.shift !== g.keys.shift || b.ctrl !== g.keys.ctrl || b.alt !== g.keys.alt) continue;
      if (g.wheel ? b.gesture === 'wheel' : b.gesture === 'drag' && b.button === g.button) return b.move;
    }
    return null;
  }
  // The camera moved: the surface draws again, and the helper is told its
  // pose at the next frame.
  function cameraMoved(s) {
    s.dirty = true;
    s.cameraMoved = true;
    schedule();
  }

  // The three buttons a pointer event names, by the number a browser's
  // pointer event gives each, with its place among them and its bit in the
  // buttons pressed.
  var pointerButtons = { 0: { name: 'Left', slot: 0, bit: 1 }, 2: { name: 'Right', slot: 1, bit: 2 }, 1: { name: 'Middle', slot: 2, bit: 4 } };
  var buttonNames = ['Left', 'Right', 'Middle'];
  // The modifier keys down at an event, Command counting as Ctrl on macOS.
  function keysOf(e) { return { shift: e.shiftKey, ctrl: e.ctrlKey || (mac && e.metaKey), alt: e.altKey }; }
  // Whether the point `cx`, `cy` in the page's points is on surface `s`.
  function onSurface(s, cx, cy) {
    var r = s.canvas.getBoundingClientRect();
    return cx >= r.left && cx <= r.right && cy >= r.top && cy <= r.bottom;
  }
  // What a pointer event at `cx`, `cy` in the page's points gives on a
  // surface drawn at `at`: where, in physical pixels from its top left and
  // as fractions of its width and height, the pixels per point, the
  // button, and the modifier keys down.
  function pointerValue(at, cx, cy, button, keys) {
    var x = cx * at.ppp - at.left, y = cy * at.ppp - at.top;
    return { x: x, y: y, u: x / at.across, v: y / at.down, ppp: at.ppp, button: button, shift: keys.shift, ctrl: keys.ctrl, alt: keys.alt };
  }
  // A button pressed over surface `s`: tracked, so its release is reported
  // wherever it happens, raising PointerDown when the node requests it,
  // and starting the drag its camera's controls bind it to.
  function surfacePressed(s, e, b) {
    if (!onSurface(s, e.clientX, e.clientY)) return;
    var keys = keysOf(e);
    s.pressed[b.slot] = true;
    if (s.wanted.down) emit(s.node, 'PointerDown', pointerValue(placeOf(s), e.clientX, e.clientY, b.name, keys));
    if (s.camera && !s.held) {
      var made = cameraMoveFor(s.camera, { button: b.name, keys: keys });
      if (made) s.held = { button: b.name, made: made, x: e.clientX, y: e.clientY };
    }
  }
  // A button pressed over surface `s` released, wherever: PointerUp when
  // the node requests it, and that button's drag ends.
  function surfaceReleased(s, e, b) {
    if (!s.pressed[b.slot]) return;
    s.pressed[b.slot] = false;
    if (s.wanted.up) emit(s.node, 'PointerUp', pointerValue(placeOf(s), e.clientX, e.clientY, b.name, keysOf(e)));
    if (s.held && s.held.button === b.name) s.held = null;
  }
  // The pointer moved, over surface `s` or with a button pressed over it
  // still down: its drag moves the camera, and a move is reported at the
  // next frame.
  function surfaceMoved(s, e) {
    s.latest = { x: e.clientX, y: e.clientY, keys: keysOf(e) };
    s.hover = onSurface(s, e.clientX, e.clientY) ? [e.clientX, e.clientY] : null;
    if (s.held) {
      var across = e.clientX - s.held.x, down = e.clientY - s.held.y;
      s.held.x = e.clientX;
      s.held.y = e.clientY;
      if (across || down) {
        cameraDrag(s.camera, s.held.made, across, down, s.box.getBoundingClientRect().height);
        cameraMoved(s);
      }
    }
    if (s.wanted.moves) {
      s.moveDue = true;
      schedule();
    }
  }
  // Raises PointerMove at the pointer's latest place over surface `s`, or
  // anywhere while a button pressed over it is down, naming the first of
  // Left, Right and Middle that is down; nothing when it has not moved
  // since the last move reported.
  function reportMove(s) {
    if (!s.latest) return;
    var held = null;
    for (var i = 0; i < 3 && held === null; i++) if (s.pressed[i]) held = buttonNames[i];
    if (!onSurface(s, s.latest.x, s.latest.y) && held === null) return;
    if (s.lastMove && s.lastMove[0] === s.latest.x && s.lastMove[1] === s.latest.y) return;
    s.lastMove = [s.latest.x, s.latest.y];
    emit(s.node, 'PointerMove', pointerValue(placeOf(s), s.latest.x, s.latest.y, held, s.latest.keys));
  }
  // The wheel turned over surface `s`: Wheel with its notches when the node
  // requests it, a line or a page counting one and 100 points one, positive
  // toward the scene; and the move the camera's controls bind it to,
  // zooming by its notches, or orbiting or panning by its points as a drag
  // would, a line counting egui's 40 points and a page the surface's
  // height. While the controls bind the wheel, the page behind does not
  // scroll. An engine turns a wheel with Shift down into a horizontal one;
  // the turn counts as the wheel gave it, as egui reads one.
  function surfaceTurned(s, e) {
    var keys = keysOf(e);
    var across = e.deltaX, down = e.deltaY;
    if (e.shiftKey && down === 0) {
      down = across;
      across = 0;
    }
    var notches = e.deltaMode === 0 ? -down / 100 : -down;
    if (s.wanted.wheel && onSurface(s, e.clientX, e.clientY)) {
      var value = pointerValue(placeOf(s), e.clientX, e.clientY, null, keys);
      value.notches = notches;
      emit(s.node, 'Wheel', value);
    }
    if (!s.camera) return;
    if (s.camera.bindings.some(function (b) { return b.gesture === 'wheel'; })) e.preventDefault();
    var made = cameraMoveFor(s.camera, { wheel: true, keys: keys });
    if (!made) return;
    if (made === 'Zoom') {
      cameraZoom(s.camera, notches);
    } else {
      var height = s.box.getBoundingClientRect().height;
      var unit = e.deltaMode === 0 ? 1 : e.deltaMode === 1 ? 40 : height;
      cameraDrag(s.camera, made, -across * unit, -down * unit, height);
    }
    cameraMoved(s);
  }
  // The value a test event of `kind` raises on a surface drawn at `at`, as
  // the egui renderer makes one: `given` names `u` and `v`, fractions of
  // the surface's width and height, with any of `button` (Left when a
  // press or release names none), `shift`, `ctrl` and `alt`, and for a
  // wheel turn its `notches`, 1 when absent, its keys compared without
  // regard to case.
  function pointerTestValue(at, kind, given) {
    if (!given || typeof given !== 'object' || Array.isArray(given)) throw new Error('a pointer test event takes a map of u and v, fractions of the surface\'s width and height, not ' + JSON.stringify(given));
    var get = function (key) {
      var found = Object.keys(given).find(function (k) { return k.toLowerCase() === key; });
      return found === undefined ? undefined : given[found];
    };
    var fraction = function (key) {
      var v = get(key);
      if (typeof v !== 'number') throw new Error('a pointer test event takes ' + key + ', a fraction of the surface');
      return v;
    };
    var held = function (key) {
      var v = get(key);
      if (v === undefined) return false;
      if (typeof v !== 'boolean') throw new Error(key + ' is true or false, not ' + JSON.stringify(v));
      return v;
    };
    var u = fraction('u'), v = fraction('v');
    var keys = { shift: held('shift'), ctrl: held('ctrl'), alt: held('alt') };
    var named = get('button'), button;
    if (kind === 'Wheel' || (kind === 'PointerMove' && (named === undefined || named === null))) button = null;
    else if (named === undefined || named === null) button = 'Left';
    else if (typeof named === 'string') {
      button = buttonNames.find(function (n) { return n.toLowerCase() === named.toLowerCase(); });
      if (!button) throw new Error(named + ' is not a pointer button: Left, Right or Middle');
    } else throw new Error('button is Left, Right or Middle, not ' + JSON.stringify(named));
    var x = u * at.across, y = v * at.down;
    var value = { x: x, y: y, u: u, v: v, ppp: at.ppp, button: button, shift: keys.shift, ctrl: keys.ctrl, alt: keys.alt };
    if (kind === 'Wheel') {
      var notches = get('notches');
      if (notches !== undefined && typeof notches !== 'number') throw new Error('notches is a number, not ' + JSON.stringify(notches));
      value.notches = notches === undefined ? 1 : notches;
    }
    return value;
  }

  // A GpuSurface: a canvas of its width and height in points, the room
  // across and the height the window leaves where either is absent, as
  // egui gives one, drawn by the page's WebGPU from what the session gave
  // it and in the window's background until then. One reading its frame
  // uniform, or with a pass run every frame, draws every frame; any other
  // when it is given draws, when a copy it draws from changes, and when it
  // resizes. It raises the pointer events its node requests, moves its
  // camera by the gestures the camera's controls bind, and one that takes
  // keys takes the focus when pressed.
  function drawSurface(node, p) {
    var w = optNumber(p, 'width'), h = optNumber(p, 'height');
    if (w !== null && !(w > 0)) throw new Error('width must be above 0 points, not ' + w);
    if (h !== null && !(h > 0)) throw new Error('height must be above 0 points, not ' + h);
    if (h === null && inScroll(node.id)) throw new Error('a GpuSurface without a width and a height takes the room its place leaves, and inside a Scroll that room has no end');
    var wanted = { down: flag(p, 'pointerDown', false), up: flag(p, 'pointerUp', false), moves: flag(p, 'pointerMove', false), wheel: flag(p, 'wheel', false) };
    var keys = flag(p, 'keys', false);
    var s = surfaceOf(node.id);
    if (s.started === null) s.started = performance.now() / 1000;
    var box = el('div', 'surface');
    if (w !== null) box.style.width = w + 'px';
    if (h !== null) box.style.height = h + 'px';
    if (keys) box.tabIndex = 0;
    box.appendChild(canvasOf(s));
    if (s.observer) s.observer.disconnect();
    s.node = node;
    s.wanted = wanted;
    s.keys = keys;
    s.box = box;
    s.hover = null;
    s.dirty = true;
    s.observer = new ResizeObserver(function () { s.dirty = true; schedule(); });
    s.observer.observe(box);
    schedule();
    return box;
  }

  // The canvas surface `s` draws on: made once, with what listens on it,
  // so drawing the surface again moves it into its new box rather than
  // making another, and its WebGPU or WebGL 2 context, with what the
  // context has, lasts. A press captures the pointer, so its moves and
  // its release come here wherever the pointer goes; a second button
  // pressed or released while one is down comes as a move naming it. It
  // senses drags where it has a camera or takes keys, as egui's surface
  // does, and else nothing. A WebGL 2 context the engine takes away is
  // waited for, and the surface built in it again when it comes back.
  function canvasOf(s) {
    if (s.canvas) return s.canvas;
    var canvas = s.canvas = el('canvas');
    senseLive(canvas, function () { return s.camera || s.keys ? 'drag' : 'hover'; });
    canvas.addEventListener('pointerdown', function (e) {
      canvas.setPointerCapture(e.pointerId);
      if (s.keys && s.box) s.box.focus();
      if (pointerButtons[e.button]) surfacePressed(s, e, pointerButtons[e.button]);
    });
    canvas.addEventListener('pointermove', function (e) {
      var chord = pointerButtons[e.button];
      if (chord) {
        if (buttonsOf(e) & chord.bit) surfacePressed(s, e, chord); else surfaceReleased(s, e, chord);
      }
      surfaceMoved(s, e);
    });
    canvas.addEventListener('pointerup', function (e) { if (pointerButtons[e.button]) surfaceReleased(s, e, pointerButtons[e.button]); });
    canvas.addEventListener('pointerleave', function () { s.hover = null; });
    canvas.addEventListener('wheel', function (e) { surfaceTurned(s, e); }, { passive: false });
    // The middle button scrolls the page by itself in some engines.
    canvas.addEventListener('mousedown', function (e) { if (e.button === 1) e.preventDefault(); });
    canvas.addEventListener('webglcontextlost', function (e) {
      e.preventDefault();
      glBackOf(s);
      surfaceProblem(s, "the engine took the surface's WebGL 2 context away; the surface draws again when it comes back");
    });
    canvas.addEventListener('webglcontextrestored', function () {
      s.glCopies = {};
      s.glTargets = null;
      s.plan = null;
      s.glRound++;
      var back = s.glBack;
      s.glBack = null;
      give(s, null, null);
      if (back) back.done();
    });
    return canvas;
  }

  // Each Canvas by node id, as canvasState makes it: the scene its node
  // last gave, which a new one replaces, its board and its animations
  // (pwrsCanvas.boardOf, pwrsCanvas.started), why its items cannot be
  // read, its node, box and canvas, and whether it waits for a frame; what
  // the person did since its last frame and whether Escape was pressed;
  // its press, click and drag as egui's interaction tracks them
  // (pwrsCanvas.interact); the buttons pressed over it, the fingers on it,
  // where the primary pointer last was and whether it is over the canvas;
  // the view's drag, the item's drag, the item the pointer was over and
  // where the last move reported was.
  var canvases = {};
  // The numbers a canvas's view moves by and how a box zoom's box is
  // drawn, as the protocol gives them.
  var viewNumbers = null;
  function seconds() { return performance.now() / 1000; }
  function canvasState(id) {
    return canvases[id] || (canvases[id] = {
      id: id, scene: undefined, board: pwrsCanvas.boardOf([]), motions: {}, problem: null, node: null, box: null, canvas: null, observer: null, dirty: true,
      input: [], escape: false, due: false, w: { potentialClick: false, potentialDrag: false, dragging: false },
      tracked: [false, false, false], touches: {}, touchLast: null, latest: null, hovering: false,
      held: null, itemDrag: null, hovered: null, lastMove: null, decideTimer: null
    });
  }
  // Takes `scene`, a Canvas's items as the helper sends them, in place of
  // what canvas `c` had: its board, and its animations as they stood when
  // the helper sent them, each begun the seconds it gives before now.
  function adoptScene(c, scene) {
    c.scene = scene;
    c.motions = {};
    c.dirty = true;
    if (!scene || scene.problem) {
      c.problem = scene ? scene.problem : null;
      c.board = pwrsCanvas.boardOf([]);
      return;
    }
    c.problem = null;
    c.board = pwrsCanvas.boardOf(scene.items);
    var now = seconds();
    (function walk(items) {
      items.forEach(function (i) {
        (i.motions || []).forEach(function (m) { (c.motions[i.id] || (c.motions[i.id] = {}))[m.animation.property] = { animation: m.animation, began: now - m.elapsed, from: m.from }; });
        walk(i.children || []);
      });
    })(scene.items);
  }
  // The canvas of node `id` with the scene its node has now.
  function canvasOfNode(id) {
    var n = find(tree, id);
    if (!n || n.kind !== 'Canvas') throw new Error('node ' + id + ' is no canvas');
    var c = canvasState(id);
    if (c.scene !== props(n).scene) adoptScene(c, props(n).scene);
    return c;
  }
  // Applies a change to a canvas's items as the helper made it: an item as
  // it now stands, its directly set properties ending their animations and
  // its Animate starting or clearing them; items added above the board's
  // own or a group's, their animations starting; items taken away with
  // theirs. The canvas draws again at the next frame.
  function changeItems(op) {
    var c = canvasOfNode(op.node), now = seconds();
    if (op.op === 'setItem') {
      var e = c.board.items[op.item.id];
      if (!e) throw new Error('canvas ' + op.node + ' has no item ' + op.item.id);
      e.name = op.item.name;
      e.shape = op.item.shape;
      e.style = op.item.style;
      e.animations = op.item.animations;
      pwrsCanvas.changed(c.motions, c.board, e.id, op.set, op.animate, now);
    } else if (op.op === 'addItems') {
      var added = pwrsCanvas.boardOf(op.items);
      Object.keys(added.items).forEach(function (id) {
        var it = added.items[id];
        if (it.parent === null) it.parent = op.group;
        c.board.items[id] = it;
      });
      var under = op.group === null ? c.board.top : c.board.items[op.group] ? c.board.items[op.group].children : null;
      if (!under) throw new Error('canvas ' + op.node + ' has no group ' + op.group);
      added.top.forEach(function (id) { under.push(id); });
      pwrsCanvas.started(c.motions, c.board, Object.keys(added.items).map(Number), now);
    } else {
      op.ids.forEach(function (id) {
        var e = c.board.items[id];
        if (!e) return;
        var siblings = e.parent === null ? c.board.top : c.board.items[e.parent] ? c.board.items[e.parent].children : [];
        var at = siblings.indexOf(id);
        if (at >= 0) siblings.splice(at, 1);
        delete c.board.items[id];
      });
      pwrsCanvas.forget(c.motions, op.ids);
    }
    c.dirty = true;
    schedule();
  }
  // A Canvas: a box of its width and height in points, the room across and
  // the height the window leaves where either is absent, as egui gives
  // one, containing the canvas its items are drawn on; the problem its items
  // are, when they cannot be read, shown in their place.
  function drawCanvas(node, p) {
    var w = optNumber(p, 'width'), h = optNumber(p, 'height');
    if (w !== null && !(w > 0)) throw new Error('width must be above 0 points, not ' + w);
    if (h !== null && !(h > 0)) throw new Error('height must be above 0 points, not ' + h);
    if (h === null && inScroll(node.id)) throw new Error('a Canvas without a width and a height takes the room its place leaves, and inside a Scroll that room has no end');
    var c = canvasOfNode(node.id);
    var box = el('div', 'surface canvas');
    if (w !== null) box.style.width = w + 'px';
    if (h !== null) box.style.height = h + 'px';
    box.appendChild(canvasElementOf(c));
    c.wanted = { down: flag(p, 'pointerDown', false), up: flag(p, 'pointerUp', false), moves: flag(p, 'pointerMove', false), wheel: flag(p, 'wheel', false) };
    c.items = { click: flag(p, 'click', false), double: flag(p, 'doubleClick', false), enter: flag(p, 'pointerEnter', false), leave: flag(p, 'pointerLeave', false), dragStart: flag(p, 'dragStart', false), dragMove: flag(p, 'dragMove', false), dragEnd: flag(p, 'dragEnd', false) };
    c.keys = flag(p, 'keys', false);
    box.addEventListener('keydown', function (e) { canvasKey(c, e); });
    if (c.problem) box.appendChild(el('div', 'label error', c.problem));
    if (c.observer) c.observer.disconnect();
    c.node = node;
    c.box = box;
    takeFocus(c);
    c.dirty = true;
    c.observer = new ResizeObserver(function () { c.dirty = true; schedule(); });
    c.observer.observe(box);
    schedule();
    return box;
  }

  // Plane transforms [a, b, c, d, e, f] as egui's canvas makes them: `s`
  // after `i`, a point going through `i` first; turns clockwise as y grows
  // downwards; an item turned, skewed and scaled about its pivot.
  var IDENTITY = [1, 0, 0, 1, 0, 0];
  function after(s, i) { return [s[0] * i[0] + s[2] * i[1], s[1] * i[0] + s[3] * i[1], s[0] * i[2] + s[2] * i[3], s[1] * i[2] + s[3] * i[3], s[0] * i[4] + s[2] * i[5] + s[4], s[1] * i[4] + s[3] * i[5] + s[5]]; }
  function moved(x, y) { return [1, 0, 0, 1, x, y]; }
  function placedBy(style, pivot) {
    var r = radians(style.rotate), turn = [Math.cos(r), Math.sin(r), -Math.sin(r), Math.cos(r), 0, 0];
    var skew = [1, Math.tan(radians(style.skew[1])), Math.tan(radians(style.skew[0])), 1, 0, 0];
    return after(after(after(after(moved(pivot[0], pivot[1]), turn), skew), [style.scale[0], 0, 0, style.scale[1], 0, 0]), moved(-pivot[0], -pivot[1]));
  }
  function pivotOf(style, b) { var o = style.origin || [0.5, 0.5]; return [b[0] + b[2] * o[0], b[1] + b[3] * o[1]]; }
  function inverted(m) {
    var det = m[0] * m[3] - m[1] * m[2];
    if (Math.abs(det) < 1e-12) return null;
    var a = m[3] / det, b = -m[1] / det, c = -m[2] / det, d = m[0] / det;
    return [a, b, c, d, -(a * m[4] + c * m[5]), -(b * m[4] + d * m[5])];
  }
  function growth(m) { return Math.sqrt(Math.abs(m[0] * m[3] - m[1] * m[2])); }
  function matrix(m) { return new DOMMatrix([m[0], m[1], m[2], m[3], m[4], m[5]]); }
  function transformTo(ctx, m) { ctx.setTransform(m[0], m[1], m[2], m[3], m[4], m[5]); }
  // The box of `points`, [x, y, width, height]; null for none.
  function spanned(points) {
    if (!points.length) return null;
    var xs = points.map(function (p) { return p[0]; }), ys = points.map(function (p) { return p[1]; });
    var x = Math.min.apply(null, xs), y = Math.min.apply(null, ys);
    return [x, y, Math.max.apply(null, xs) - x, Math.max.apply(null, ys) - y];
  }
  function cornersOf(b) { return [[b[0], b[1]], [b[0] + b[2], b[1]], [b[0] + b[2], b[1] + b[3]], [b[0], b[1] + b[3]]]; }
  function unionOf(boxes) { return spanned([].concat.apply([], boxes.map(function (b) { return [[b[0], b[1]], [b[0] + b[2], b[1] + b[3]]]; }))); }
  // The part of box `b` within `area`: null for everywhere, 'nowhere', or
  // a box; null where no room is left of it.
  function cutTo(b, area) {
    if (!b || area === 'nowhere') return null;
    if (!area) return b;
    var left = Math.max(b[0], area[0]), top = Math.max(b[1], area[1]), right = Math.min(b[0] + b[2], area[0] + area[2]), bottom = Math.min(b[1] + b[3], area[1] + area[3]);
    return right >= left && bottom >= top ? [left, top, right - left, bottom - top] : null;
  }
  // A color #rrggbbaa as Canvas 2D takes one.
  function cssOf(hex) { var c = [1, 3, 5, 7].map(function (i) { return parseInt(hex.slice(i, i + 2), 16); }); return 'rgba(' + c[0] + ', ' + c[1] + ', ' + c[2] + ', ' + c[3] / 255 + ')'; }
  // The theme color the stylesheet's variable `name` gives, as #rrggbbaa,
  // read as egui's visuals give it; opaque black where none is read.
  var colorProbe = document.createElement('canvas').getContext('2d');
  function themeColor(name) {
    colorProbe.fillStyle = '#000000';
    colorProbe.fillStyle = getComputedStyle(document.documentElement).getPropertyValue(name).trim() || '#000000';
    var v = colorProbe.fillStyle, m = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(v);
    if (v.charAt(0) === '#') return v + 'ff';
    var hex = function (n) { return ('0' + Number(n).toString(16)).slice(-2); };
    return m ? '#' + hex(m[1]) + hex(m[2]) + hex(m[3]) + hex(Math.round((m[4] === undefined ? 1 : Number(m[4])) * 255)) : '#000000ff';
  }
  // The theme's text color, which an item giving neither Fill nor Stroke is
  // drawn in.
  function themeText() { return themeColor('--text'); }
  // The metrics of the face a canvas's text is set in, in its own units,
  // as the stylesheet gives them; null where they were not read.
  function textFace() {
    var style = getComputedStyle(document.documentElement);
    var read = function (key) { var v = parseFloat(style.getPropertyValue('--face-' + key)); return isFinite(v) ? v : null; };
    var face = { units: read('units'), ascent: read('ascent'), descent: read('descent'), gap: read('gap') };
    return face.units !== null && face.ascent !== null && face.descent !== null && face.gap !== null ? face : null;
  }

  // A canvas's view as egui's ViewState has it, on canvas `c`: `view`,
  // none while the items' coordinates are the canvas's own, and while a
  // move is under way where it ends; `motion`, a move { from, to, began,
  // ease } under way; and `held`, the drag a press began. The view drawn
  // at `now` on a canvas `size` points across and down, whether a move
  // eases it and whether it moves, eased or dragged, a move ended where it
  // is, and an eased move to `to` from the view drawn now.
  function viewShown(c, now, size) {
    var m = c.motion;
    if (m && now - m.began < m.ease.seconds) return pwrsCanvas.toward(m.from, m.to, pwrsCanvas.ease(m.ease.easing, (now - m.began) / m.ease.seconds), size);
    return c.view || null;
  }
  function viewEasing(c, now) { return !!(c.motion && now - c.motion.began < c.motion.ease.seconds); }
  function viewMoving(c, now) { return viewEasing(c, now) || !!c.held; }
  function settleView(c, now, size) {
    if (!c.motion) return;
    c.view = viewShown(c, now, size);
    c.motion = null;
  }
  // Moves canvas `c`'s view to `to` from the view drawn at `now`, over
  // `ease`; a canvas without a view is drawn under the default one, its own
  // middle at zoom 1, so its move starts there.
  function easeView(c, to, ease, now, size) {
    settleView(c, now, size);
    var from = c.view || { center: null, zoom: 1, minZoom: to.minZoom, maxZoom: to.maxZoom, bindings: to.bindings };
    c.motion = { from: from, to: to, began: now, ease: ease };
    c.view = to;
  }

  // The box, [x, y, width, height] in the items' coordinates, of what `fit`
  // names, "All" or names, as canvas `c`'s last frame drew it under the
  // view drawn then, from the boxes a fit counts; null before it is drawn
  // or where it drew nothing `fit` names. And `view` fitted to that box,
  // or as it is without one.
  function drawnBox(c, fit) {
    if (!c.frame || !c.place) return null;
    var named = function (h) { return h.span && fit.some(function (n) { return sameItemName(n, h.name); }); };
    var px = fit === 'All' ? unionOf(c.frame.spans) : unionOf(c.frame.hits.filter(named).map(function (h) { return h.span; }));
    if (!px) return null;
    var ppp = c.place.ppp, size = sizeOf(c), under = c.drawnView || { center: null, zoom: 1 };
    var low = pwrsCanvas.toItems(under, [px[0] / ppp, px[1] / ppp], size), high = pwrsCanvas.toItems(under, [(px[0] + px[2]) / ppp, (px[1] + px[3]) / ppp], size);
    return [low[0], low[1], high[0] - low[0], high[1] - low[1]];
  }
  // Whether two item names are one, compared without regard to the case of
  // ASCII letters, as egui compares them.
  function sameItemName(a, b) {
    var fold = function (s) { return s.replace(/[A-Z]/g, function (ch) { return ch.toLowerCase(); }); };
    return fold(a) === fold(b);
  }
  function fittedView(c, view, fit, size) {
    var b = drawnBox(c, fit);
    if (!b) return view;
    var to = pwrsCanvas.copy(view);
    pwrsCanvas.fit(to, b, size);
    return to;
  }
  // Canvas `c`'s view as a call returns it and the page reports it: as
  // drawn at `now`, its center the one its size gives it once it has one,
  // a farthest of none as "Infinity", and whether it moves.
  function answeredView(c, now, size) {
    var shown = size ? viewShown(c, now, size) : c.view || null;
    if (!shown) return { view: null, moving: false };
    var center = size ? pwrsCanvas.centerIn(shown, size) : shown.center;
    return { view: { center: center, zoom: shown.zoom, minZoom: shown.minZoom, maxZoom: shown.maxZoom === Infinity ? 'Infinity' : shown.maxZoom }, moving: viewMoving(c, now) };
  }
  // A view as the helper sends one, a farthest of "Infinity" for none.
  function takenView(spec) {
    var v = pwrsCanvas.copy(spec);
    if (v.maxZoom === 'Infinity') v.maxZoom = Infinity;
    if (v.minZoom === 'Infinity') v.minZoom = Infinity;
    return v;
  }
  // Reports canvas `c`'s view to the helper when it is drawn other than it
  // was last reported, as the helper stores it for its calls and a page
  // loaded again.
  function reportView(c) {
    if (!c.view || !c.place) return;
    var size = [c.place.across / c.place.ppp, c.place.down / c.place.ppp];
    var answer = answeredView(c, seconds(), size), written = JSON.stringify(answer);
    if (written === c.reportedView) return;
    c.reportedView = written;
    post({ t: 'view', node: c.id, view: answer.view, moving: answer.moving });
  }

  // Offscreen canvases a frame draws translucent and clipped groups into,
  // reused from frame to frame.
  var layerPool = [];

  // Where canvas `c` lies, its top left and size in the window's physical
  // pixels and the display's pixels a point, as its frames place it; false
  // while it is not laid out.
  function placeCanvas(c) {
    var canvas = c.canvas;
    if (!canvas || !canvas.isConnected) return false;
    var r = canvas.getBoundingClientRect(), ppp = window.devicePixelRatio || 1;
    var left = Math.round(r.left * ppp), top = Math.round(r.top * ppp);
    var across = Math.round(r.right * ppp) - left, down = Math.round(r.bottom * ppp) - top;
    if (across < 1 || down < 1) return false;
    c.place = { left: left, top: top, across: across, down: down, ppp: ppp };
    return true;
  }

  // Draws one frame of canvas `c`, placed, at `now`, at its size in
  // physical pixels, its items under its view, as egui's canvas draws one,
  // and records where each item was drawn: the hits named items take events
  // by, topmost last, and the boxes a fit counts. Returns whether it must
  // draw again at the next frame: an animation, a view's eased move or a
  // picture still loading.
  function canvasFrame(c, now) {
    var canvas = c.canvas, across = c.place.across, down = c.place.down, ppp = c.place.ppp;
    if (canvas.width !== across) canvas.width = across;
    if (canvas.height !== down) canvas.height = down;
    var size = [across / ppp, down / ppp];
    var shown = viewShown(c, now, size);
    var world = after([ppp, 0, 0, ppp, 0, 0], shown ? pwrsCanvas.transform(shown, size) : IDENTITY);
    var ctx = c.ctx || (c.ctx = canvas.getContext('2d'));
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, across, down);
    var making = { c: c, now: now, ppp: ppp, text: themeText(), face: textFace(), font: getComputedStyle(document.documentElement).getPropertyValue('--font-ui').trim() || 'sans-serif', hits: [], spans: [], problems: [], pending: false, depth: 0 };
    drawItems(making, ctx, c.board.top, world, pwrsCanvas.nothingPassed, { paths: [], area: null }, null);
    c.frame = { hits: making.hits, spans: making.spans };
    c.drawnView = shown;
    if (c.fitPending && c.view) {
      c.view = fittedView(c, c.view, c.fitPending, size);
      c.fitPending = null;
      c.dirty = true;
    }
    if (making.problems.length && c.reported !== making.problems[0]) {
      c.reported = making.problems[0];
      post({ t: 'error', context: 'draw', text: 'Canvas ' + (c.node && c.node.name ? c.node.name : '') + ': ' + making.problems[0] });
    }
    return making.pending || pwrsCanvas.moving(c.motions, now) || viewEasing(c, now);
  }

  // Draws the items `ids` in their order, the shown ones, each under what
  // its group passes on, inside `clips`, named by its own name or the
  // nearest named group's.
  function drawItems(m, ctx, ids, world, passed, clips, named) {
    var order = ids.map(function (id) { return m.c.board.items[id]; }).filter(function (e) { return e && e.style.visible; });
    order.sort(function (a, b) { return a.style.order - b.style.order; });
    order.forEach(function (e) { drawItem(m, ctx, e, world, passed, clips, e.name || named); });
  }
  // Item `e` with its animations' values at the frame's moment set on a
  // copy, reading here any animation's start that the theme sets; `fill`
  // reads the starts, as drawing does, and measuring leaves them.
  function animated(m, e, passed, fill) {
    var own = m.c.motions[e.id];
    if (!own || !Object.keys(own).length) return e;
    var item = { id: e.id, name: e.name, shape: Object.assign({}, e.shape), style: Object.assign({}, e.style), animations: e.animations, children: e.children, parent: e.parent };
    Object.keys(own).forEach(function (p) {
      if (fill && (own[p].from === null || own[p].from === undefined)) own[p].from = pwrsCanvas.firstDrawn(e, passed, p, m.text);
      var v = pwrsCanvas.motionValue(own[p], m.now);
      if (v !== null) pwrsCanvas.setValue(item, p, v);
    });
    return item;
  }
  function drawItem(m, ctx, e, world, passed, clips, name) {
    var item = animated(m, e, passed, true), sh = item.shape;
    var inner = pwrsCanvas.under(passed, item.style);
    if (sh.kind === 'Group') { drawItemGroup(m, ctx, e, item, world, inner, clips, name); return; }
    if (sh.kind === 'Text') { drawText(m, ctx, item, world, inner, clips, name); return; }
    if (sh.kind === 'Image') { drawPicture(m, ctx, item, world, clips, name); return; }
    var segments = pwrsCanvas.outline(sh);
    if (!segments) return;
    var own = pwrsCanvas.bounds(segments, IDENTITY);
    drawShape(m, ctx, item, segments, own, after(world, own ? placedBy(item.style, pivotOf(item.style, own)) : IDENTITY), inner, clips, name);
  }
  // A shape's fill and stroke, `full` placing its outline `segments` in
  // pixels, `own` their box, which a gradient spreads over; the box a fit
  // counts for it joins the frame's, and a named one takes events where it
  // is drawn: inside its closed subpaths, filled or not, inside the rest
  // where its fill paints, and within half its stroke or 5 points of its
  // outline.
  function drawShape(m, ctx, item, segments, own, full, passed, clips, name) {
    var p = pwrsCanvas.paints(item, passed, m.text);
    var path = new Path2D(pwrsCanvas.pathData(segments));
    var width = passed.strokeWidth === null || passed.strokeWidth === undefined ? 1 : passed.strokeWidth;
    ctx.globalAlpha = item.style.opacity;
    if (p.fill) fillPath(ctx, path, p.fill, own, full);
    if (p.stroke && width > 0) {
      transformTo(ctx, full);
      strokeAs(ctx, passed, width);
      ctx.strokeStyle = cssOf(p.stroke);
      ctx.stroke(path);
    }
    ctx.globalAlpha = 1;
    var span = cutTo(pwrsCanvas.visualBox(segments, full, p.stroke ? width : 0), clips.area);
    if (span) m.spans.push(span);
    if (name) {
      var pixels = new Path2D(), inside = null;
      pixels.addPath(path, matrix(full));
      if (p.fill) inside = pixels;
      else {
        var closedPart = pwrsCanvas.closed(segments);
        if (closedPart.length) {
          inside = new Path2D();
          inside.addPath(new Path2D(pwrsCanvas.pathData(closedPart)), matrix(full));
        }
      }
      m.hits.push({ name: name, path: pixels, inside: inside, reach: Math.max(width * growth(full) / 2, 5 * m.ppp), clips: clips.paths, span: span });
    }
  }
  // A stroke's width, dash, caps and joins as a group passes them on,
  // lyon's miter limit of 4 among them.
  function strokeAs(ctx, passed, width) {
    ctx.lineWidth = width;
    ctx.setLineDash(passed.dash || []);
    ctx.lineDashOffset = 0;
    ctx.lineCap = (passed.cap || 'Butt').toLowerCase();
    ctx.lineJoin = (passed.join || 'Miter').toLowerCase();
    ctx.miterLimit = 4;
  }
  // Fills `path`, in the item's own coordinates, with `paint`: one color,
  // or a gradient in fractions of the item's box `own`, the path moved
  // into those fractions so the gradient lies there as egui lays it.
  function fillPath(ctx, path, paint, own, full) {
    if (paint.color !== undefined) {
      transformTo(ctx, full);
      ctx.fillStyle = cssOf(paint.color);
      ctx.fill(path, 'nonzero');
      return;
    }
    if (!own || !(own[2] > 0) || !(own[3] > 0)) return;
    var box = [own[2], 0, 0, own[3], own[0], own[1]];
    var inFractions = new Path2D();
    inFractions.addPath(path, matrix(inverted(box)));
    transformTo(ctx, after(full, box));
    ctx.fillStyle = gradientOf(ctx, paint);
    ctx.fill(inFractions, 'nonzero');
  }
  // A gradient's Canvas 2D gradient in fractions of the item's box, its
  // stops blending premultiplied; a line with no length takes its first
  // color, as egui's does.
  function gradientOf(ctx, paint) {
    var colors = paint.linear || paint.radial;
    if (paint.linear && paint.from[0] === paint.to[0] && paint.from[1] === paint.to[1]) return cssOf(colors[0]);
    var g = paint.linear ? ctx.createLinearGradient(paint.from[0], paint.from[1], paint.to[0], paint.to[1]) : ctx.createRadialGradient(paint.center[0], paint.center[1], 0, paint.center[0], paint.center[1], paint.radius);
    pwrsCanvas.gradientStops(colors, paint.stops).forEach(function (s) { g.addColorStop(Math.min(1, Math.max(0, s[0])), cssOf(s[1])); });
    return g;
  }
  // An offscreen canvas the size of canvas `m.c`'s, cleared, for a group
  // drawn as one layer, and its return to the pool.
  function layerOf(m) {
    var layer = layerPool.pop() || { canvas: document.createElement('canvas') };
    if (layer.canvas.width !== m.c.place.across) layer.canvas.width = m.c.place.across;
    if (layer.canvas.height !== m.c.place.down) layer.canvas.height = m.c.place.down;
    layer.ctx = layer.ctx || layer.canvas.getContext('2d');
    layer.ctx.setTransform(1, 0, 0, 1, 0, 0);
    layer.ctx.clearRect(0, 0, layer.canvas.width, layer.canvas.height);
    return layer;
  }
  // A canvas Group item's items, placed by its offset, turn, skew and scale
  // about its pivot inside `world`, drawn into a layer of their own when it
  // is translucent or clips, which is drawn back through its clip at its
  // opacity.
  function drawItemGroup(m, ctx, e, item, world, passed, clips, name) {
    var inner = after(world, groupPlacing(m, e, item));
    var clipSegments = item.shape.clip ? pwrsCanvas.outline(item.shape.clip) : null, clipPath = null, innerClips = clips;
    if (clipSegments) {
      clipPath = new Path2D();
      clipPath.addPath(new Path2D(pwrsCanvas.pathData(clipSegments)), matrix(inner));
      var clipBox = pwrsCanvas.bounds(clipSegments, inner);
      innerClips = { paths: clips.paths.concat([clipPath]), area: clipBox ? cutTo(clipBox, clips.area) || 'nowhere' : 'nowhere' };
    }
    if (item.style.opacity >= 1 && !clipPath) { drawItems(m, ctx, e.children, inner, passed, innerClips, name); return; }
    var layer = layerOf(m);
    drawItems(m, layer.ctx, e.children, inner, passed, innerClips, name);
    ctx.save();
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    if (clipPath) ctx.clip(clipPath, 'nonzero');
    ctx.globalAlpha = item.style.opacity;
    ctx.drawImage(layer.canvas, 0, 0);
    ctx.restore();
    layerPool.push(layer);
  }
  // What places a Group's items in its group's coordinates: its offset,
  // then its turn, skew and scale about the point its Origin gives in
  // fractions of the box around its items, shown or not, or its own 0,0.
  function groupPlacing(m, e, item) {
    var pivot = [0, 0];
    if (item.style.origin) {
      var around = unionOf(e.children.map(function (id) { return placedBox(m, id, IDENTITY); }).filter(Boolean));
      if (around) pivot = pivotOf(item.style, around);
    }
    return after(moved(item.shape.x, item.shape.y), placedBy(item.style, pivot));
  }
  // The exact box of what item `id` draws, its animations' values set on
  // it, once `under` places its group's coordinates; null for nothing.
  function placedBox(m, id, under) {
    var e = m.c.board.items[id];
    if (!e) return null;
    var item = animated(m, e, pwrsCanvas.nothingPassed, false), sh = item.shape;
    var quad = function (own) { var full = after(under, placedBy(item.style, pivotOf(item.style, own))); return spanned(cornersOf(own).map(function (p) { return pwrsCanvas.applied(full, p); })); };
    if (sh.kind === 'Group') { var inner = after(under, groupPlacing(m, e, item)); return unionOf(e.children.map(function (c) { return placedBox(m, c, inner); }).filter(Boolean)); }
    if (sh.kind === 'Text') return quad(textBox(m, sh));
    if (sh.kind === 'Image') return quad([sh.x, sh.y, sh.width, sh.height]);
    var segments = pwrsCanvas.outline(sh), own = segments ? pwrsCanvas.bounds(segments, IDENTITY) : null;
    return own ? pwrsCanvas.bounds(segments, after(under, placedBy(item.style, pivotOf(item.style, own)))) : null;
  }
  // The rows a Text takes in the UI face as egui lays them out, one for
  // each line of its text, at egui's row height in the face's metrics, or
  // 1.2 times its size with the baseline at its size where the face's
  // metrics were not read.
  function textRows(m, sh) {
    var size = sh.size === null || sh.size === undefined ? 14 : sh.size;
    var row = m.face ? pwrsCanvas.textRow(m.face, size, m.ppp) : { height: size * 1.2, baseline: size };
    return pwrsCanvas.textLines(sh.text, row, m.ppp);
  }
  // The box a Text takes: as wide as its widest line measured, and as tall
  // as its rows.
  function textBox(m, sh) {
    var size = sh.size === null || sh.size === undefined ? 14 : sh.size;
    colorProbe.font = size + 'px ' + m.font;
    var rows = textRows(m, sh), width = 0;
    rows.lines.forEach(function (line) { width = Math.max(width, colorProbe.measureText(line.text).width); });
    return [sh.x, sh.y, width, rows.height];
  }
  // A Text, filled and stroked as its paints give, each line's baseline
  // where egui puts it, a gradient filling it through a layer in fractions
  // of its box; it takes events across its box.
  function drawText(m, ctx, item, world, passed, clips, name) {
    var sh = item.shape, size = sh.size === null || sh.size === undefined ? 14 : sh.size;
    var own = textBox(m, sh), rows = textRows(m, sh);
    var full = after(world, placedBy(item.style, pivotOf(item.style, own)));
    var p = pwrsCanvas.paints(item, passed, m.text);
    var width = passed.strokeWidth === null || passed.strokeWidth === undefined ? 1 : passed.strokeWidth;
    var write = function (target, fill) {
      transformTo(target, full);
      target.font = size + 'px ' + m.font;
      target.textAlign = 'left';
      target.textBaseline = 'alphabetic';
      rows.lines.forEach(function (line) {
        if (fill) target.fillText(line.text, sh.x, sh.y + line.baseline);
        else target.strokeText(line.text, sh.x, sh.y + line.baseline);
      });
    };
    ctx.globalAlpha = item.style.opacity;
    if (p.fill && p.fill.color !== undefined) { ctx.fillStyle = cssOf(p.fill.color); write(ctx, true); }
    else if (p.fill && own[2] > 0 && own[3] > 0) {
      var layer = layerOf(m);
      layer.ctx.fillStyle = '#ffffff';
      write(layer.ctx, true);
      layer.ctx.globalCompositeOperation = 'source-in';
      transformTo(layer.ctx, after(full, [own[2], 0, 0, own[3], own[0], own[1]]));
      layer.ctx.fillStyle = gradientOf(layer.ctx, p.fill);
      layer.ctx.fillRect(-1, -1, 3, 3);
      layer.ctx.globalCompositeOperation = 'source-over';
      ctx.save();
      ctx.setTransform(1, 0, 0, 1, 0, 0);
      ctx.drawImage(layer.canvas, 0, 0);
      ctx.restore();
      layerPool.push(layer);
    }
    if (p.stroke && width > 0) { strokeAs(ctx, passed, width); ctx.strokeStyle = cssOf(p.stroke); write(ctx, false); }
    ctx.globalAlpha = 1;
    boxedHit(m, own, full, clips, name);
  }
  // Where a Text's or an Image's box `own`, placed by `full`, was drawn:
  // the box a fit counts joins the frame's, and a named one takes events
  // across it.
  function boxedHit(m, own, full, clips, name) {
    var quad = cornersOf(own).map(function (p) { return pwrsCanvas.applied(full, p); });
    var span = cutTo(spanned(quad), clips.area);
    if (span) m.spans.push(span);
    if (name) m.hits.push({ name: name, quad: quad, clips: clips.paths, span: span });
  }
  // An Image's picture, loaded from the address the helper gives it,
  // drawn in its box at its opacity once loaded, the frame drawn again
  // when it is; one that cannot be drawn is reported.
  function drawPicture(m, ctx, item, world, clips, name) {
    var sh = item.shape, own = [sh.x, sh.y, sh.width, sh.height];
    var full = after(world, placedBy(item.style, pivotOf(item.style, own)));
    boxedHit(m, own, full, clips, name);
    if (typeof sh.src !== 'string') { m.problems.push((item.name ? 'Image ' + item.name : 'an Image') + ': its picture could not be drawn: the window has no such picture'); return; }
    var c = m.c, img = (c.images || (c.images = {}))[sh.src];
    if (!img) {
      img = c.images[sh.src] = new Image();
      img.onload = function () { c.dirty = true; schedule(); };
      img.onerror = function () { c.dirty = true; schedule(); };
      img.src = sh.src;
    }
    if (!img.complete) { m.pending = true; return; }
    if (!img.naturalWidth) { m.problems.push((item.name ? 'Image ' + item.name : 'an Image') + ': its picture could not be drawn'); return; }
    transformTo(ctx, full);
    ctx.globalAlpha = item.style.opacity;
    ctx.drawImage(img, sh.x, sh.y, sh.width, sh.height);
    ctx.globalAlpha = 1;
  }
  // The name of the topmost named item canvas `c` drew at `x`, `y` in its
  // pixels, as its last frame drew them: inside what counts as an
  // outline's inside, within a hit's reach of its outline, or inside a
  // text's or picture's box, every clip around it containing the point too.
  var hitProbe = document.createElement('canvas').getContext('2d');
  function itemAt(c, x, y) {
    var hits = c.frame ? c.frame.hits : [];
    hitProbe.setTransform(1, 0, 0, 1, 0, 0);
    for (var i = hits.length - 1; i >= 0; i--) {
      var h = hits[i];
      if (!h.clips.every(function (p) { return hitProbe.isPointInPath(p, x, y, 'nonzero'); })) continue;
      if (h.quad) { if (inQuad(h.quad, x, y)) return h.name; continue; }
      if (h.inside && hitProbe.isPointInPath(h.inside, x, y, 'nonzero')) return h.name;
      hitProbe.lineWidth = 2 * h.reach;
      hitProbe.lineCap = 'round';
      hitProbe.lineJoin = 'round';
      hitProbe.setLineDash([]);
      if (hitProbe.isPointInStroke(h.path, x, y)) return h.name;
    }
    return null;
  }
  function inQuad(q, x, y) {
    var side = function (a, b) { return (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]); };
    var s = [side(q[0], q[1]), side(q[1], q[2]), side(q[2], q[3]), side(q[3], q[0])];
    return s.every(function (v) { return v >= 0; }) || s.every(function (v) { return v <= 0; });
  }

  // The pointer as egui tracks it for the window (pwrsCanvas.pointer), from
  // the primary pointer's presses, moves and releases, taken before any
  // element has them, so a canvas reads each release's click; a button no
  // longer down, its release having gone outside the window, let go at the
  // next event. The modifier keys and the space bar as they were at the
  // latest key or pointer event; and Escape, with which egui ends every
  // drag. A press the page makes on the widget egui's hit test gives it to
  // (actOut) is the person's own, already taken.
  var pointerNow = pwrsCanvas.pointer(), heldKeys = { shift: false, ctrl: false, alt: false }, spaceHeld = false;
  var lastRelease = { event: null, clicks: [] };
  function pointerTaken(e) {
    if (!e.isTrusted) return;
    heldKeys = keysOf(e);
    if (!e.isPrimary) return;
    var b = pointerButtons[e.button], at = [e.clientX, e.clientY], t = seconds();
    for (var slot = 0; slot < 3; slot++) {
      if (pointerNow.down[slot] && (buttonsOf(e) & [1, 2, 4][slot]) === 0 && !(b && b.slot === slot)) letGo(e, slot, at, t);
    }
    if (e.type === 'pointermove') pwrsCanvas.moved(pointerNow, at);
    if (e.type === 'pointercancel') {
      for (var i = 0; i < 3; i++) if (pointerNow.down[i]) letGo(e, i, at, t);
    } else if (b && (e.type === 'pointerdown' || (e.type === 'pointermove' && (buttonsOf(e) & b.bit) !== 0))) pwrsCanvas.pressed(pointerNow, b.slot, at, t);
    else if (b) letGo(e, b.slot, at, t);
  }
  function letGo(e, slot, at, t) {
    if (lastRelease.event !== e) lastRelease = { event: e, clicks: [] };
    lastRelease.clicks[slot] = pwrsCanvas.released(pointerNow, slot, at, t);
  }
  // Whether event `e` let the button in `slot` go, and the click it let it
  // go with, or null.
  function letGoBy(e, slot) { return lastRelease.event === e && lastRelease.clicks[slot] !== undefined; }
  function clickOf(e, slot) { return letGoBy(e, slot) ? lastRelease.clicks[slot] : null; }
  ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'].forEach(function (type) { document.addEventListener(type, pointerTaken, true); });

  // The window's widgets as egui's hit test takes them (pwrsCanvas.hitTest),
  // so a press, its drag and its click go to the widget egui's would give
  // them to, a few points around it included. Each senses what the egui
  // widget it stands for senses. Buttons, links, tabs, check boxes and
  // options, a tree's rows and arrows, a table's rows, a menu's entries, a
  // chart legend's entries and a document's links take clicks. Drag values,
  // text fields, text labels, as egui's selectable labels do, a chart's
  // plot, a color picker's square and bar and a table column's edges take
  // clicks and drags. A slider and a split's divider take drags. A Canvas
  // and a GpuSurface sense what their node and view bind (liveSenses). A
  // check box's or an option's text is its own, as in egui, and no label of
  // its own.
  var LABELS = '.label:not(.check .label), .markdown :is(p, h1, h2, h3, h4, h5, h6, li, pre, th, td)';
  var SENSES = [
    ['button, .check, .tree .treeitem, .tree .arrow, .table tbody tr, .markdown a, .chart .u-legend .u-series > th', 'click'],
    ['input.drag, .field, ' + LABELS + ', .chart .u-over, .picker canvas, .table .edge', 'both'],
    ['input[type=range], .split > .divider', 'drag']
  ];
  var SENSED = '[data-sense], ' + SENSES.map(function (s) { return s[0]; }).join(', ');
  // The boxes a widget's box is cut to: those the page scrolls, a popup's
  // and a menu's.
  var CLIPS = SCROLLERS + ', .popup, .menu';
  var liveSenses = new WeakMap();
  function senseOf(e) {
    var live = liveSenses.get(e);
    if (live) return live();
    for (var i = 0; i < SENSES.length; i++) if (e.matches(SENSES[i][0])) return SENSES[i][1];
    return 'hover';
  }
  // Marks element `e` as a widget sensing what `sense` returns as it is
  // asked, 'click', 'drag', 'both' or 'hover'.
  function senseLive(e, sense) {
    e.dataset.sense = 'live';
    liveSenses.set(e, sense);
  }
  function enabledOf(e) { return !(e.disabled || e.closest('.off, [disabled]') || (e.matches('.check') && e.querySelector('input:disabled'))); }
  // The box egui gives widget `e`, [left, top, right, bottom] in the page's
  // points: a label's around its text, as egui's is around its galley, and
  // any other's around its border. It is cut, as egui cuts a widget's to
  // its clip rect, to the inside of each box it scrolls or pops up in and to
  // the window, so a widget scrolled out of sight has a negative box.
  function hitBoxOf(e) {
    var r = e.getBoundingClientRect();
    if (e.matches(LABELS)) {
      var range = document.createRange();
      range.selectNodeContents(e);
      var text = range.getBoundingClientRect();
      if (text.width > 0 || text.height > 0) r = text;
    }
    var box = [r.left, r.top, r.right, r.bottom];
    var cut = function (left, top, right, bottom) { box = [Math.max(box[0], left), Math.max(box[1], top), Math.min(box[2], right), Math.min(box[3], bottom)]; };
    for (var s = e.parentElement && e.parentElement.closest(CLIPS); s; s = s.parentElement && s.parentElement.closest(CLIPS)) {
      var c = s.getBoundingClientRect();
      cut(c.left + s.clientLeft, c.top + s.clientTop, c.left + s.clientLeft + s.clientWidth, c.top + s.clientTop + s.clientHeight);
    }
    cut(0, 0, window.innerWidth, window.innerHeight);
    return box;
  }
  // The window's widgets back to front, as egui's hit test takes them, each
  // { id, el, layer, rect, click, drag, enabled } with its place in the list
  // as its id. First come the window's own area, as large as the window and
  // sensing nothing, and its widgets in the order the page draws them. Then
  // each popup and menu open, in the order they opened, is a layer over the
  // ones before: its own area, sensing clicks as egui's interactable areas
  // do, then its widgets. The list is read again at each press and release,
  // and once a frame as the pointer moves.
  var hitList = null;
  function hitWidgets() {
    if (hitList) return hitList;
    var list = [];
    var add = function (e, layer, sense) {
      if (!e.getClientRects().length) return;
      list.push({ id: list.length, el: e, layer: layer, rect: hitBoxOf(e), click: sense === 'click' || sense === 'both', drag: sense === 'drag' || sense === 'both', enabled: enabledOf(e) });
    };
    list.push({ id: 0, el: root, layer: 0, rect: [0, 0, window.innerWidth, window.innerHeight], click: false, drag: false, enabled: true });
    Array.prototype.forEach.call(root.querySelectorAll(SENSED), function (e) { add(e, 0, senseOf(e)); });
    Array.prototype.forEach.call(popups.children, function (area, i) {
      add(area, i + 1, 'click');
      Array.prototype.forEach.call(area.querySelectorAll(SENSED), function (e) { add(e, i + 1, senseOf(e)); });
    });
    hitList = list;
    requestAnimationFrame(function () { hitList = null; });
    return list;
  }
  // What egui's hit test finds at `at`: the widgets a press there would
  // click and drag, or null, and those holding the pointer, as elements.
  function hitsAt(at) {
    var list = hitWidgets(), h = pwrsCanvas.hitTest(list, at, pwrsCanvas.interactRadius);
    var of = function (id) { return id === null ? null : list[id].el; };
    return { click: of(h.click), drag: of(h.drag), contains: h.contains.map(of) };
  }
  // The widget drawn at or around `element`, or null.
  function widgetOf(element) { return element && element.closest ? element.closest(SENSED) : null; }

  // egui's interaction over the window (egui interaction.rs). At a press the
  // hit test names the widgets it may click and drag. That widget is dragged
  // once the press is decidedly a drag, or at once when it senses drags
  // alone, or when the pointer is neither inside it nor given to it for a
  // click any more. A release by egui's click rule clicks the widget the
  // press may click. While a widget is clicked or dragged it alone is
  // hovered, and otherwise the hit test's widgets are. A widget a press may
  // click or drag is drawn held, as egui draws one a button is down on.
  var engaged = { click: null, drag: null, dragged: null, clicked: null, hovered: [], held: [] };
  function engage(e, hits, t) {
    var b = pointerButtons[e.button];
    var down = !!b && (e.type === 'pointerdown' || (e.type === 'pointermove' && (buttonsOf(e) & b.bit) !== 0));
    var up = !!b && !down && e.type !== 'pointerdown' && e.type !== 'pointercancel';
    engaged.clicked = null;
    if (down) {
      if (!engaged.click) engaged.click = hits.click;
      if (!engaged.drag) engaged.drag = hits.drag;
    } else if (up || e.type === 'pointercancel') {
      if (up && clickOf(e, b.slot) && engaged.click) engaged.clicked = engaged.click;
      engaged.click = engaged.drag = engaged.dragged = null;
    }
    var anyDown = pointerNow.down.some(Boolean), w = engaged.drag;
    if (!engaged.dragged && w && enabledOf(w)) {
      var decided = anyDown && !pwrsCanvas.clickable(pointerNow, t);
      if (senseOf(w) !== 'both' || decided || !(hits.contains.indexOf(w) >= 0 || hits.click === w)) engaged.dragged = w;
    }
    if (!pwrsCanvas.clickable(pointerNow, t)) engaged.click = null;
    if (!anyDown) engaged.click = engaged.drag = null;
    var only = engaged.clicked || engaged.dragged;
    var hovered = only ? [only] : [hits.click, hits.drag];
    look(hovered, 'hovered');
    look([engaged.click, engaged.drag], 'held');
    fringeCursor(e, hovered[0] || hovered[1] || null);
  }
  // The pointer's shape, which follows the widget egui's hit test picks, as
  // egui's does. Where that widget is not the one under the pointer, as a
  // few points beside it or a drag gone past it, the root shows the
  // widget's own cursor over everything (runtime.css, data-cursor).
  // Elsewhere each element shows its own. The root changes only when that
  // widget does (fringe), since a change there styles every element again.
  var fringe = null;
  function fringeCursor(e, widget) {
    var under = widgetOf(document.elementFromPoint(e.clientX, e.clientY));
    var beside = widget && under !== widget && !(under && widget.contains(under)) ? widget : null;
    if (beside === fringe) return;
    fringe = beside;
    var html = document.documentElement;
    // The widget's own cursor is read with the root's taken off. A text
    // field's and a label's is the text cursor, which egui shows over them
    // and an engine shows over their text alone.
    delete html.dataset.cursor;
    var cursor = beside ? getComputedStyle(beside).cursor : 'auto';
    if (cursor === 'auto' && beside && beside.matches('.field, ' + LABELS)) cursor = 'text';
    if (cursor === 'auto') {
      html.style.removeProperty('--fringe-cursor');
      return;
    }
    html.style.setProperty('--fringe-cursor', cursor);
    html.dataset.cursor = '';
  }
  // Draws the elements of `list` with class `name`, and no others.
  function look(list, name) {
    var now = list.filter(function (x, i) { return x && list.indexOf(x) === i; });
    engaged[name].forEach(function (x) { if (now.indexOf(x) < 0) x.classList.remove(name); });
    now.forEach(function (x) { x.classList.add(name); });
    engaged[name] = now;
  }
  document.documentElement.addEventListener('pointerleave', function (e) {
    if (!e.isTrusted || pointerNow.down.some(Boolean)) return;
    look([], 'hovered');
    fringeCursor(e, null);
  });

  // The buttons held when `e` came: its own, with those a test's input
  // holds, as the bridge keeps them (window.pwrs.__buttonsOf).
  function buttonsOf(e) {
    return window.pwrs.__buttonsOf(e);
  }

  // A press goes where egui's hit test gives it. Where that is a widget
  // other than the one the engine found under the pointer, the engine's
  // widget never has it, and the page makes the press on egui's (actOut).
  // A widget the page works itself takes that press as it takes a person's.
  // A text field takes the keyboard's focus, the cursor where egui puts it
  // and the selection a drag then makes, and a slider takes the value at
  // the pointer as it moves.
  var redirect = null;
  window.addEventListener('pointerdown', function (e) {
    if (!e.isTrusted || !e.isPrimary) return;
    redirect = null;
    hitList = null;
    var hits = hitsAt([e.clientX, e.clientY]), to = hits.click || hits.drag, own = widgetOf(e.target);
    if (!to || (own && (own === hits.click || own === hits.drag))) return;
    e.preventDefault();
    e.stopPropagation();
    pointerTaken(e);
    engage(e, hits, seconds());
    redirect = { el: to, button: e.button, anchor: null };
    actOut(e, to);
  }, true);
  // Every click of the pointer is egui's: a left release within egui's
  // click distance and time of its press clicks the widget the press may
  // click (clickWidget), and the engine's own clicks of the pointer are
  // dropped. A click a key makes, as Space on a button does, stays the
  // engine's, as do those the page's own click makes as it is dispatched.
  var emulating = false;
  ['click', 'dblclick', 'auxclick'].forEach(function (type) {
    window.addEventListener(type, function (e) {
      if (!e.isTrusted || e.detail === 0 || emulating) return;
      e.preventDefault();
      e.stopPropagation();
    }, true);
  });
  document.addEventListener('selectstart', function (e) { if (redirect) e.preventDefault(); }, true);
  ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'].forEach(function (type) {
    document.addEventListener(type, function (e) {
      if (!e.isTrusted || !e.isPrimary) return;
      if (e.type !== 'pointermove') hitList = null;
      engage(e, hitsAt([e.clientX, e.clientY]), seconds());
      if (redirect) follow(e);
      clickWidget(e);
      rightClicked(e);
    }, true);
  });
  // A left release that clicks by egui's rule clicks the widget the press
  // may click, with a double-click at the second click, carrying the keys
  // held. A label, a field and a slider take no click, and a check box's or
  // an option's click is its own box's.
  function clickWidget(e) {
    var click = clickOf(e, 0), to = engaged.clicked;
    if (!click || !to || to.matches(LABELS + ', input, textarea')) return;
    var init = {
      bubbles: true, cancelable: true, composed: true, view: window, detail: click.count, button: 0, buttons: 0,
      clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, metaKey: e.metaKey
    };
    var target = to.matches('.check') ? to.querySelector('input') || to : to;
    emulating = true;
    try {
      target.dispatchEvent(new MouseEvent('click', init));
      if (click.count === 2) target.dispatchEvent(new MouseEvent('dblclick', init));
    } finally {
      emulating = false;
    }
  }
  // The press `e` made on widget `to`, which egui's hit test gave it to.
  function actOut(e, to) {
    var init = {
      bubbles: true, cancelable: true, composed: true, view: window, pointerId: e.pointerId, pointerType: e.pointerType, isPrimary: true, width: e.width, height: e.height, pressure: e.pressure,
      button: e.button, buttons: buttonsOf(e), clientX: e.clientX, clientY: e.clientY, screenX: e.screenX, screenY: e.screenY, ctrlKey: e.ctrlKey, shiftKey: e.shiftKey, altKey: e.altKey, metaKey: e.metaKey
    };
    to.dispatchEvent(new PointerEvent('pointerdown', init));
    if (e.button !== 0 || !enabledOf(to)) return;
    if (to.matches('.field')) {
      to.focus({ preventScroll: true });
      if (to.tagName !== 'TEXTAREA' && ['text', 'search', 'url', 'tel', 'password'].indexOf(to.type) < 0) return;
      redirect.anchor = caretAt(to, [e.clientX, e.clientY]);
      to.setSelectionRange(redirect.anchor, redirect.anchor);
    } else if (to.matches('input[type=range]')) slideTo(to, e.clientX);
  }
  // A move or the release of a press the page made on egui's widget: a text
  // field's selection from where the press put the cursor, and a slider's
  // value, then its change once the press is let go.
  function follow(e) {
    var r = redirect, to = r.el, b = pointerButtons[e.button], own = pointerButtons[r.button];
    var ended = e.type === 'pointercancel' || (!!b && b.slot === own.slot && (buttonsOf(e) & own.bit) === 0 && e.type !== 'pointerdown');
    if (r.button === 0 && enabledOf(to)) {
      if (r.anchor !== null) {
        var at = caretAt(to, [e.clientX, e.clientY]);
        to.setSelectionRange(Math.min(r.anchor, at), Math.max(r.anchor, at), at < r.anchor ? 'backward' : 'forward');
      } else if (to.matches('input[type=range]')) slideTo(to, e.clientX);
    }
    if (!ended) return;
    redirect = null;
    if (to.matches('input[type=range]')) to.dispatchEvent(new Event('change', { bubbles: true }));
  }
  // The place in text field `f`'s text nearest `at`, as egui puts the cursor
  // at a press: on the line nearest it, at the edge between characters
  // nearest it across, in the field's own font.
  var caretMeasure = document.createElement('canvas').getContext('2d');
  function caretAt(f, at) {
    var s = getComputedStyle(f), r = f.getBoundingClientRect();
    caretMeasure.font = s.font || [s.fontStyle, s.fontWeight, s.fontSize, s.fontFamily].join(' ');
    var shown = f.type === 'password' ? f.value.replace(/[^]/g, '•') : f.value;
    var lines = f.tagName === 'TEXTAREA' ? shown.split('\n') : [shown];
    var high = parseFloat(s.lineHeight) || parseFloat(s.fontSize) * 1.2;
    var top = r.top + f.clientTop + parseFloat(s.paddingTop) - f.scrollTop;
    var row = Math.min(Math.max(Math.floor((at[1] - top) / high), 0), lines.length - 1);
    var x = at[0] - (r.left + f.clientLeft + parseFloat(s.paddingLeft) - f.scrollLeft);
    var line = lines[row], best = 0, gap = Infinity;
    for (var i = 0; i <= line.length; i++) {
      var d = Math.abs(caretMeasure.measureText(line.slice(0, i)).width - x);
      if (d < gap) { gap = d; best = i; }
    }
    for (var k = 0; k < row; k++) best += lines[k].length + 1;
    return best;
  }
  // Moves slider `bar` to the value at `x` across, as egui's slider takes
  // the value at the pointer: along the rail between the handle's ends, on
  // its steps, raising input as the engine's own slider does.
  function slideTo(bar, x) {
    var r = bar.getBoundingClientRect(), handle = 2 * cssPoints('--interact-h') / 2.5;
    var share = Math.min(Math.max((x - r.left - handle / 2) / Math.max(r.width - handle, 1), 0), 1);
    var before = bar.value;
    bar.value = String(Number(bar.min) + (Number(bar.max) - Number(bar.min)) * share);
    if (bar.value !== before) bar.dispatchEvent(new Event('input', { bubbles: true }));
  }

  // A context menu opens as the egui renderer opens one, at a right click:
  // the right button let go within egui's click distance and time of its
  // press. The click is on the window's own layer, not over an open menu or
  // popup. Of the widgets with a menu whose boxes hold that place, the
  // smallest's menu opens there. The engine's own context menu never shows,
  // and its contextmenu event, which a right press or release, the Menu key
  // and Shift+F10 raise as the engine has them, opens nothing.
  window.addEventListener('contextmenu', function (e) { e.preventDefault(); }, true);
  function rightClicked(e) {
    if (e.type === 'pointercancel' || !clickOf(e, 1)) return;
    var at = [e.clientX, e.clientY], top = document.elementFromPoint(at[0], at[1]);
    if (top && popups.contains(top)) return;
    var menus = [], best = null;
    (function walk(n) { if (n.kind === 'ContextMenu') menus.push(n); (n.children || []).forEach(walk); })(tree);
    menus.forEach(function (m) {
      var name = props(m).for, target = typeof name === 'string' ? findNamed(tree, name) : null;
      var drawn = target ? document.querySelector('[data-node="' + target.id + '"]') : null;
      if (!drawn || !drawn.getClientRects().length) return;
      var r = drawn.getBoundingClientRect(), area = r.width * r.height;
      if (at[0] >= r.left && at[0] <= r.right && at[1] >= r.top && at[1] <= r.bottom && (!best || area < best.area)) best = { menu: m, area: area };
    });
    if (best) openMenu(best.menu, at[0], at[1], 0, null);
  }

  document.addEventListener('keydown', function (e) {
    heldKeys = keysOf(e);
    if (e.key === ' ') spaceHeld = true;
    if (e.key !== 'Escape') return;
    Object.keys(canvases).forEach(function (id) {
      canvases[id].escape = true;
      canvases[id].due = true;
    });
    schedule();
  }, true);
  document.addEventListener('keyup', function (e) {
    heldKeys = keysOf(e);
    if (e.key === ' ') spaceHeld = false;
  }, true);
  window.addEventListener('blur', function () {
    heldKeys = { shift: false, ctrl: false, alt: false };
    spaceHeld = false;
  });
  function sizeOf(c) { return c.place ? [c.place.across / c.place.ppp, c.place.down / c.place.ppp] : [0, 0]; }
  // The point `at` in the page's points, from canvas `c`'s top left in
  // points; whether it is on the canvas; the canvas's middle in the page's
  // points; and the item its last frame drew there.
  function localOf(c, at) { var p = c.place; return [(at[0] * p.ppp - p.left) / p.ppp, (at[1] * p.ppp - p.top) / p.ppp]; }
  function onCanvas(c, at) { var r = c.canvas.getBoundingClientRect(); return at[0] >= r.left && at[0] <= r.right && at[1] >= r.top && at[1] <= r.bottom; }
  function canvasMiddle(c) { var r = c.canvas.getBoundingClientRect(); return [(r.left + r.right) / 2, (r.top + r.bottom) / 2]; }
  function itemAtClient(c, at) { return c.place && c.frame ? itemAt(c, at[0] * c.place.ppp - c.place.left, at[1] * c.place.ppp - c.place.top) : null; }
  // What a canvas event at `at` in the page's points gives: a GpuSurface
  // pointer event's values, `item`, or the item the frame drew there when
  // none is given, and where it is in the items' coordinates under the
  // view the frame was drawn with.
  function canvasValue(c, at, button, keys, item) {
    var v = pointerValue(c.place, at[0], at[1], button, keys);
    v.item = item === undefined ? itemAt(c, v.x, v.y) : item;
    return withScene(c, v);
  }
  function withScene(c, v) {
    var at = [v.x / c.place.ppp, v.y / c.place.ppp], s = c.drawnView ? pwrsCanvas.toItems(c.drawnView, at, sizeOf(c)) : at;
    v.scenex = s[0];
    v.sceney = s[1];
    return v;
  }
  // What canvas `c` senses as egui's canvas does: drags, where its items or
  // its view take them; clicks, where they take those or it takes the
  // focus; the focus, where its node requests keys or its view binds a key
  // or a drag with the space bar down; and whether its view binds a gesture
  // `test` picks out from its written parts.
  var BUTTON_GESTURES = ['Left', 'Right', 'Middle'], POINTER_GESTURES = ['Left', 'Right', 'Middle', 'Wheel', 'Pinch', 'DoubleClick', 'RightDoubleClick', 'MiddleDoubleClick'];
  function viewBinds(c, test) { return !!(c.view && Object.keys(c.view.bindings || {}).some(function (g) { return test(g.split('+')); })); }
  function itemDrags(c) { return c.items.dragStart || c.items.dragMove || c.items.dragEnd; }
  function sensesDrag(c) { return itemDrags(c) || viewBinds(c, function (parts) { return BUTTON_GESTURES.indexOf(parts[parts.length - 1]) >= 0; }); }
  function viewTakesKeys(c) {
    return viewBinds(c, function (parts) {
      var last = parts[parts.length - 1];
      return POINTER_GESTURES.indexOf(last) < 0 || (BUTTON_GESTURES.indexOf(last) >= 0 && parts.indexOf('Space') >= 0);
    });
  }
  function focusable(c) { return c.keys || viewTakesKeys(c); }
  function sensesClick(c) { return sensesDrag(c) || c.items.click || c.items.double || focusable(c) || viewBinds(c, function (parts) { return /DoubleClick$/.test(parts[parts.length - 1]); }); }
  function takeFocus(c) {
    if (!c.box) return;
    if (focusable(c)) c.box.tabIndex = 0; else c.box.removeAttribute('tabindex');
  }

  // The canvas canvas `c` draws on: made once, with what listens on it, so
  // drawing the canvas again moves it into its new box. What the person
  // does on it waits for the canvas's next frame, as egui's input waits for
  // its next pass. That is each press and release with its button, place
  // and keys, a release with the click the window's pointer counted, each
  // wheel turn, a trackpad's pinch, and the fingers on it. It is also where
  // the primary pointer last was and whether it is over the canvas. A press
  // captures the pointer, so its moves and its release come here wherever it
  // goes; a second button pressed or released while one is down comes as a
  // move naming it; and a press past egui's click time requests a frame then.
  // It senses what egui's canvas senses: clicks and drags where it takes
  // drags, clicks where it takes only those, and else neither.
  function canvasElementOf(c) {
    if (c.canvas) return c.canvas;
    var canvas = c.canvas = el('canvas');
    senseLive(canvas, function () { return sensesDrag(c) ? 'both' : sensesClick(c) ? 'click' : 'hover'; });
    var due = function () { c.due = true; schedule(); };
    var press = function (e, b) {
      c.input.push({ press: b.slot, at: [e.clientX, e.clientY], keys: keysOf(e), over: onCanvas(c, [e.clientX, e.clientY]) });
      clearTimeout(c.decideTimer);
      c.decideTimer = setTimeout(due, pwrsCanvas.clickSeconds * 1000 + 1);
    };
    var release = function (e, slot) { c.input.push({ release: slot, at: [e.clientX, e.clientY], keys: keysOf(e), click: clickOf(e, slot) }); };
    canvas.addEventListener('pointerdown', function (e) {
      if (e.pointerType === 'touch') touched(c, e, true);
      if (e.isPrimary) {
        c.hovering = true;
        canvas.setPointerCapture(e.pointerId);
        if (c.box && focusable(c)) c.box.focus({ preventScroll: true });
        c.latest = [e.clientX, e.clientY];
        if (pointerButtons[e.button]) press(e, pointerButtons[e.button]);
      }
      due();
    });
    canvas.addEventListener('pointermove', function (e) {
      if (e.pointerType === 'touch') touched(c, e, true);
      if (e.isPrimary) {
        if (!canvas.hasPointerCapture(e.pointerId)) c.hovering = true;
        c.latest = [e.clientX, e.clientY];
        var chord = pointerButtons[e.button];
        if (chord && (buttonsOf(e) & chord.bit) !== 0) press(e, chord);
        else if (chord) release(e, chord.slot);
      }
      due();
    });
    var up = function (e) {
      if (e.pointerType === 'touch') touched(c, e, false);
      if (e.isPrimary) {
        c.latest = [e.clientX, e.clientY];
        if (e.type === 'pointercancel') {
          for (var i = 0; i < 3; i++) if (letGoBy(e, i)) release(e, i);
        } else if (pointerButtons[e.button]) release(e, pointerButtons[e.button].slot);
      }
      due();
    };
    canvas.addEventListener('pointerup', up);
    canvas.addEventListener('pointercancel', up);
    canvas.addEventListener('pointerenter', function (e) {
      if (!e.isPrimary) return;
      c.hovering = true;
      c.latest = [e.clientX, e.clientY];
      due();
    });
    canvas.addEventListener('pointerleave', function (e) {
      if (!e.isPrimary) return;
      c.hovering = false;
      c.latest = [e.clientX, e.clientY];
      due();
    });
    canvas.addEventListener('wheel', function (e) { canvasWheel(c, e); due(); }, { passive: false });
    // The middle button scrolls the page by itself in some engines.
    canvas.addEventListener('mousedown', function (e) { if (e.button === 1) e.preventDefault(); });
    canvas.addEventListener('gesturestart', function (e) {
      e.preventDefault();
      c.gestureScale = 1;
    });
    canvas.addEventListener('gesturechange', function (e) {
      e.preventDefault();
      c.input.push({ zoom: e.scale / (c.gestureScale || 1) });
      c.gestureScale = e.scale;
      due();
    });
    canvas.addEventListener('gestureend', function (e) { e.preventDefault(); });
    return canvas;
  }
  // The drag starting on canvas `c` at `now`, as egui's view takes it: the
  // first of Left, Right and Middle that is down whose drag its controls
  // bind with the modifier keys and the space bar down now, unless a left
  // drag without the space bar begins where a named item that takes drags
  // was drawn; the drag counts from where the press began. Returns whether
  // the view took it.
  function takeDrag(c, now, size) {
    var origin = pointerNow.origin || canvasMiddle(c);
    for (var i = 0; i < 3; i++) {
      if (!pointerNow.down[i]) continue;
      var made = pwrsCanvas.moveFor(c.view, pwrsCanvas.written(buttonNames[i], { shift: heldKeys.shift, ctrl: heldKeys.ctrl, alt: heldKeys.alt, space: spaceHeld }));
      if (!made) continue;
      if (i === 0 && !spaceHeld && itemDrags(c) && itemAtClient(c, origin) !== null) continue;
      settleView(c, now, size);
      var from = localOf(c, origin);
      c.held = { slot: i, made: made, origin: origin, from: from, last: origin, places: [[now, from]] };
      return true;
    }
    return false;
  }
  // The view's drag let go at `now`, the pointer at `at`: a glide goes on
  // at the speed the pointer had over the last moments, slowing as
  // Leaflet's does; a box zoom fits what its box covers; each eased. And a
  // drag let go where it is, as Escape and two fingers let one go.
  function endHeld(c, h, at, now, size) {
    dropHeld(c);
    var to = null, ease = null;
    if (h.made === 'Glide') {
      var velocity = pwrsCanvas.releaseVelocity(h.places, now, viewNumbers.glideWindow), g = velocity ? pwrsCanvas.glide(velocity, viewNumbers) : null;
      if (g) {
        to = pwrsCanvas.copy(c.view);
        pwrsCanvas.pan(to, g.by[0], g.by[1], size);
        ease = { seconds: g.seconds, easing: viewNumbers.glideEasing };
      }
    } else if (h.made === 'BoxZoom') {
      to = pwrsCanvas.boxed(c.view, h.from, localOf(c, at), size);
      ease = { seconds: viewNumbers.stepSeconds, easing: viewNumbers.stepEasing };
    }
    if (to) easeView(c, to, ease, now, size);
  }
  function dropHeld(c) {
    boxHide(c);
    if (c.held) c.dirty = true;
    c.held = null;
  }
  // The step `made` from the view drawn at `now`, about `at` in points on
  // the canvas, an arrow's pan from `arrow`, a fit to `fit`, every item as
  // the last frame drew them, eased over a step's seconds.
  function stepView(c, made, at, arrow, fit, now, size) {
    settleView(c, now, size);
    var to = c.view ? pwrsCanvas.stepped(c.view, made, at, arrow, fit, size, viewNumbers) : null;
    if (!to) return;
    easeView(c, to, { seconds: viewNumbers.stepSeconds, easing: viewNumbers.stepEasing }, now, size);
    c.dirty = true;
  }
  // The wheel turned over canvas `c`, for its next frame: how far it moves
  // a page's content, in points, a line counting 100 and a page the
  // canvas's height, its notches, a line or a page counting one and 100
  // points one, and its modifier keys; while the view's controls bind the
  // wheel the page behind does not scroll. An engine turns a wheel with
  // Shift down into a sideways one; the turn counts as the wheel gave it,
  // as egui reads one.
  function canvasWheel(c, e) {
    var across = e.deltaX, down = e.deltaY;
    if (e.shiftKey && down === 0) {
      down = across;
      across = 0;
    }
    if (viewBinds(c, function (parts) { return parts[parts.length - 1] === 'Wheel'; })) e.preventDefault();
    var unit = e.deltaMode === 0 ? 1 : e.deltaMode === 1 ? viewNumbers.scrollStep : sizeOf(c)[1];
    c.input.push({ wheel: { by: [-across * unit, -down * unit], notches: e.deltaMode === 0 ? -down / 100 : -down, keys: keysOf(e) } });
  }
  // A key pressed while canvas `c` has the focus, which its view takes from
  // the canvas's key handler when its controls bind it, its step made at
  // the next frame, or when it is the space bar, without Ctrl or Alt,
  // while they bind a drag with the space bar down.
  function canvasKey(c, e) {
    var k = pressedKeys(e);
    if (!k || !c.view) return;
    var kept = k.key === 'Space' && !k.ctrl && !k.alt ? viewBinds(c, function (parts) { return parts.indexOf('Space') >= 0; }) : pwrsCanvas.moveFor(c.view, pwrsCanvas.written(k.key, k)) !== null;
    if (!kept) return;
    e.preventDefault();
    e.stopPropagation();
    if (k.key === 'Space') return;
    c.input.push({ key: k });
    c.due = true;
    schedule();
  }
  // A box zoom's box over the items, from where the press began to the
  // pointer, both in the page's points, as Leaflet draws it, within the
  // canvas; and its going.
  function boxShow(c, from, to) {
    var r = c.box.getBoundingClientRect(), d = c.zoomBox || (c.zoomBox = el('div', 'zoombox'));
    if (d.parentNode !== c.box) c.box.appendChild(d);
    d.style.borderWidth = viewNumbers.boxBorderWidth + 'px';
    d.style.borderColor = cssOf(viewNumbers.boxBorder);
    d.style.background = cssOf(viewNumbers.boxFill);
    d.style.left = (Math.min(from[0], to[0]) - r.left) + 'px';
    d.style.top = (Math.min(from[1], to[1]) - r.top) + 'px';
    d.style.width = Math.abs(to[0] - from[0]) + 'px';
    d.style.height = Math.abs(to[1] - from[1]) + 'px';
  }
  function boxHide(c) { if (c.zoomBox) c.zoomBox.remove(); }
  // A finger on canvas `c` down or moved, or lifted; and the fingers since
  // the canvas's last frame, as egui's multi-touch gives them: their
  // middle, the factor their spread from it grew by and how far the middle
  // moved, none at the frame the fingers change; null for fewer than two.
  function touched(c, e, down) {
    if (down) c.touches[e.pointerId] = [e.clientX, e.clientY];
    else delete c.touches[e.pointerId];
  }
  function touchOf(c) {
    var ids = Object.keys(c.touches).sort();
    if (ids.length < 2) {
      c.touchLast = null;
      return null;
    }
    var points = ids.map(function (id) { return c.touches[id]; }), n = points.length;
    var middle = [points.reduce(function (s, p) { return s + p[0]; }, 0) / n, points.reduce(function (s, p) { return s + p[1]; }, 0) / n];
    var spread = points.reduce(function (s, p) { return s + Math.hypot(p[0] - middle[0], p[1] - middle[1]); }, 0) / n;
    var last = c.touchLast, same = last && last.ids === ids.join(',');
    c.touchLast = { ids: ids.join(','), middle: middle, spread: spread };
    return { middle: middle, zoom: same && last.spread > 0 ? spread / last.spread : 1, across: same ? [middle[0] - last.middle[0], middle[1] - last.middle[1]] : [0, 0] };
  }

  // One frame of canvas `c` at `now`, in the order egui's frame takes a
  // canvas. What the person did since the last sets its press, click and
  // drag, egui's hit test at the pointer giving the press, and its view
  // moves by them. It draws, if anything changed, and the events its node
  // requests are raised, each naming the item the new frame drew at the
  // pointer. Returns whether it must draw again at the next frame.
  function canvasPass(c, now) {
    var input = c.input, escape = c.escape;
    c.input = [];
    c.escape = false;
    c.due = false;
    if (!c.items || !placeCanvas(c)) return false;
    var size = sizeOf(c), fitAll = drawnBox(c, 'All');
    var presses = input.filter(function (i) { return i.press !== undefined || i.release !== undefined; });
    var over = !!(c.hovering && c.latest && onCanvas(c, c.latest));
    var hits = c.latest ? hitsAt(c.latest) : { click: null, drag: null, contains: [] };
    var frame = { events: presses, escape: escape, inside: hits.contains.indexOf(c.canvas) >= 0, hitClick: hits.click === c.canvas, hitDrag: hits.drag === c.canvas, click: sensesClick(c), drag: sensesDrag(c) };
    var it = pwrsCanvas.interact(c.w, pointerNow, frame, now);
    var tookDrag = steerCanvas(c, input, it, presses, escape, over, fitAll, now, size);
    if (c.dirty || c.animating) {
      c.dirty = false;
      c.animating = canvasFrame(c, now);
    }
    reportView(c);
    canvasEvents(c, input, it, presses, over, tookDrag);
    return c.animating || c.dirty || !!c.held;
  }

  // Canvas `c`'s view moved by a frame's input, as egui's view::steer
  // moves it: two fingers let a drag go; a drag starting is the view's
  // when its controls bind it; the view's drag draws its box or moves it
  // by the pointer's move since the last frame and records the pointer's
  // place at each frame, which a glide measures, and once the drag ends it
  // glides on or zooms to its box, while Escape lets it go where it is;
  // over the canvas, each wheel turn and a pinch move it as the controls
  // bind them; a double-click of a button steps it unless a left one lands
  // where a named item that takes double-clicks was drawn; and each key it
  // took steps it about the canvas's middle. Returns whether it took a
  // drag starting in this frame.
  var ARROWS = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  function steerCanvas(c, input, it, presses, escape, over, fitAll, now, size) {
    var touch = touchOf(c), took = false;
    if (!c.view) {
      dropHeld(c);
      return false;
    }
    if (touch) dropHeld(c);
    if (!c.held && !touch && it.started) took = takeDrag(c, now, size);
    var h = c.held, at = c.latest || (h && h.last);
    if (h && escape) dropHeld(c);
    else if (h && c.w.dragging && pointerNow.down[h.slot]) {
      if (h.made === 'BoxZoom') boxShow(c, h.origin, at);
      else if (at[0] !== h.last[0] || at[1] !== h.last[1]) {
        pwrsCanvas.dragged(c.view, h.made, at[0] - h.last[0], at[1] - h.last[1], h.from, size, viewNumbers);
        c.dirty = true;
      }
      h.places.push([now, localOf(c, at)]);
      h.places = h.places.filter(function (s) { return now - s[0] <= 2 * viewNumbers.glideWindow; });
      h.last = at;
    } else if (h) endHeld(c, h, at, now, size);
    if (over) {
      var pointer = localOf(c, c.latest);
      input.forEach(function (i) {
        var made = i.wheel ? pwrsCanvas.moveFor(c.view, pwrsCanvas.written('Wheel', i.wheel.keys)) : null;
        if (!made) return;
        settleView(c, now, size);
        pwrsCanvas.wheel(c.view, made, i.wheel.by, pointer, size, viewNumbers);
        c.dirty = true;
      });
      var pinch = pwrsCanvas.moveFor(c.view, 'Pinch');
      if (pinch) {
        var spread = input.reduce(function (f, i) { return i.zoom ? f * i.zoom : f; }, 1);
        var factor = touch ? touch.zoom * spread : spread, across = touch ? touch.across : [0, 0], middle = touch ? localOf(c, touch.middle) : pointer;
        if (factor !== 1 || across[0] !== 0 || across[1] !== 0) {
          settleView(c, now, size);
          pwrsCanvas.pan(c.view, across[0], across[1], size);
          if (pinch === 'Zoom') pwrsCanvas.zoomAbout(c.view, factor, middle, size);
          c.dirty = true;
        }
      }
    }
    var clickAt = c.latest || canvasMiddle(c);
    for (var b = 0; b < 3; b++) {
      if (!pwrsCanvas.clickBy(it, presses, b, 2)) continue;
      if (b === 0 && c.items.double && itemAtClient(c, clickAt) !== null) continue;
      var stepped = pwrsCanvas.moveFor(c.view, pwrsCanvas.written(b === 0 ? 'DoubleClick' : buttonNames[b] + 'DoubleClick', heldKeys));
      if (stepped) stepView(c, stepped, localOf(c, clickAt), null, fitAll, now, size);
    }
    input.forEach(function (i) {
      var made = i.key ? pwrsCanvas.moveFor(c.view, pwrsCanvas.written(i.key.key, i.key)) : null;
      if (made) stepView(c, made, [size[0] / 2, size[1] / 2], ARROWS[i.key.key] || null, fitAll, now, size);
    });
    return took;
  }

  // The events a frame of canvas `c` raises, in egui's order, the frame
  // drawn. The named item under the pointer is entered and left as the
  // pointer or the items move, the pointer counting while it is over the
  // canvas or in the canvas's drag or click. Click goes to the named item at
  // a left click, and DoubleClick at a left double-click. A left drag the
  // view did not take, begun on a named item, raises DragStart where the
  // press began, DragMove as the pointer moves and DragEnd where the drag
  // ends. Then come the pointer events: a button pressed over the canvas,
  // tracked so its release is reported wherever it happens, and each wheel
  // turn over it. Its move comes once a frame, over it or while a button
  // pressed over it is down, naming the first of Left, Right and Middle
  // that is down.
  function canvasEvents(c, input, it, presses, over, tookDrag) {
    var keys = heldKeys, at = c.latest || canvasMiddle(c);
    if (c.items.enter || c.items.leave) {
      var hoverAt = c.latest && (c.w.dragging || it.clicked || over) ? c.latest : null;
      var under = hoverAt ? itemAtClient(c, hoverAt) : null;
      if (under !== c.hovered) {
        var where = hoverAt || at;
        if (c.hovered !== null && c.items.leave) emit(c.node, 'PointerLeave', canvasValue(c, where, null, keys, c.hovered));
        if (under !== null && c.items.enter) emit(c.node, 'PointerEnter', canvasValue(c, where, null, keys, under));
        c.hovered = under;
      }
    }
    var clicked = pwrsCanvas.clickBy(it, presses, 0), item = clicked ? itemAtClient(c, at) : null;
    if (item !== null && c.items.click) emit(c.node, 'Click', canvasValue(c, at, 'Left', keys, item));
    if (item !== null && c.items.double && pwrsCanvas.clickBy(it, presses, 0, 2)) emit(c.node, 'DoubleClick', canvasValue(c, at, 'Left', keys, item));
    if (itemDrags(c)) {
      if (it.started && pointerNow.down[0] && !tookDrag) {
        var origin = pointerNow.origin || canvasMiddle(c), dragged = itemAtClient(c, origin);
        if (dragged !== null) {
          if (c.items.dragStart) emit(c.node, 'DragStart', canvasValue(c, origin, 'Left', keys, dragged));
          c.itemDrag = { item: dragged, last: origin };
        }
      }
      if (c.itemDrag && c.w.dragging && c.latest && (c.latest[0] !== c.itemDrag.last[0] || c.latest[1] !== c.itemDrag.last[1])) {
        c.itemDrag.last = c.latest;
        if (c.items.dragMove) emit(c.node, 'DragMove', canvasValue(c, c.latest, 'Left', keys, c.itemDrag.item));
      }
      if (c.itemDrag && it.stopped) {
        if (c.items.dragEnd) emit(c.node, 'DragEnd', canvasValue(c, at, 'Left', keys, c.itemDrag.item));
        c.itemDrag = null;
      }
    }
    if (c.wanted.down || c.wanted.up || c.wanted.moves || c.wanted.wheel) {
      input.forEach(function (i) {
        if (i.press !== undefined && i.over && over) {
          c.tracked[i.press] = true;
          if (c.wanted.down) emit(c.node, 'PointerDown', canvasValue(c, i.at, buttonNames[i.press], i.keys));
        } else if (i.release !== undefined && c.tracked[i.release]) {
          c.tracked[i.release] = false;
          if (c.wanted.up) emit(c.node, 'PointerUp', canvasValue(c, i.at, buttonNames[i.release], i.keys));
        } else if (i.wheel && c.wanted.wheel && over) {
          var turn = canvasValue(c, at, null, i.wheel.keys);
          turn.notches = i.wheel.notches;
          emit(c.node, 'Wheel', turn);
        }
      });
      var held = null;
      for (var k = 0; k < 3 && held === null; k++) if (c.tracked[k]) held = buttonNames[k];
      if (c.wanted.moves && c.latest && (over || held !== null) && !(c.lastMove && c.lastMove[0] === c.latest[0] && c.lastMove[1] === c.latest[1])) {
        c.lastMove = c.latest;
        emit(c.node, 'PointerMove', canvasValue(c, c.latest, held, keys));
      }
    }
  }

  // A drop-down's button, as egui's combo box draws one, in a text field's
  // look: the text chosen and a triangle pointing down, a combo box's width
  // at least unless `narrow`.
  function comboButton(text, enabled, narrow) {
    var b = el('button', 'btn frame well combo' + (narrow ? ' narrow' : ''));
    b.appendChild(el('span', 'chosen', text));
    b.appendChild(el('span', 'arrow'));
    b.disabled = !enabled;
    b.setAttribute('role', 'combobox');
    b.setAttribute('aria-haspopup', 'listbox');
    b.setAttribute('aria-expanded', 'false');
    return b;
  }
  // Works a drop-down's items from its `button`: a click opens them under
  // it as selectable labels, the one `current` returns selected, no
  // narrower than the button. From the keyboard Space and Enter open them
  // as a click does and the arrows do too; while they are open the arrows,
  // Home and End move along them, Enter or Space chooses the one moved to,
  // and Tab closes them, as Escape and a press elsewhere do. `pick` takes
  // the row chosen.
  function attachChoices(button, node, items, current, pick) {
    var hot = -1, spaceTaken = false;
    var move = function (i) {
      if (!popup || popup.anchor !== button || !items.length) return;
      var options = popup.element.children;
      if (options[hot]) options[hot].classList.remove('hot');
      hot = Math.min(Math.max(i, 0), items.length - 1);
      options[hot].classList.add('hot');
      options[hot].scrollIntoView({ block: 'nearest' });
      button.setAttribute('aria-activedescendant', options[hot].id);
    };
    var open = function () {
      var box = el('div', 'popup choices');
      box.setAttribute('role', 'listbox');
      box.id = 'choices-' + node.id + '-' + (button.dataset.node || '');
      button.setAttribute('aria-controls', box.id);
      var at = current();
      items.forEach(function (text, i) {
        var o = el('button', 'btn frame selectable' + (at === i ? ' picked' : ''), text);
        o.tabIndex = -1;
        o.id = box.id + '-' + i;
        o.setAttribute('role', 'option');
        o.setAttribute('aria-selected', at === i ? 'true' : 'false');
        o.onclick = function () { closePopup(); pick(i); };
        box.appendChild(o);
      });
      hot = -1;
      openPopup(button, box, node, true);
      move(at === null ? 0 : at);
    };
    button.onclick = function () { if (popup && popup.anchor === button) closePopup(); else open(); };
    button.onkeydown = function (e) {
      if (!(popup && popup.anchor === button)) {
        if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { open(); e.preventDefault(); }
        return;
      }
      if (e.key === 'ArrowDown') move(hot + 1);
      else if (e.key === 'ArrowUp') move(hot - 1);
      else if (e.key === 'Home') move(0);
      else if (e.key === 'End') move(items.length - 1);
      else if (e.key === 'Enter' || e.key === ' ') {
        var chosenRow = hot;
        spaceTaken = e.key === ' ';
        closePopup();
        if (chosenRow >= 0) pick(chosenRow);
      } else if (e.key === 'Tab') { closePopup(); return; }
      else return;
      e.preventDefault();
    };
    button.onkeyup = function (e) { if (e.key === ' ' && spaceTaken) { spaceTaken = false; e.preventDefault(); } };
  }
  // An editable drop-down, as the egui renderer draws one: a text field and
  // beside it a narrow drop-down of the items, one of which chosen becomes
  // the text.
  function drawEditableCombo(node, p) {
    var items = needStrings(p, 'items');
    need(p, 'text', 'string');
    var enabled = flag(p, 'enabled', true);
    var box = el('span', 'editable' + (enabled ? '' : ' off'));
    var field = el('input', 'field');
    field.type = 'text';
    field.spellcheck = false;
    field.value = values[node.id];
    field.disabled = !enabled;
    field.dataset.node = node.id + '-input';
    field.oninput = function () { changed(node, field.value); };
    var button = comboButton('', enabled, true);
    button.dataset.node = node.id + '-choices';
    button.setAttribute('aria-label', 'Choices');
    attachChoices(button, node, items, function () { var at = items.indexOf(values[node.id]); return at < 0 ? null : at; }, function (i) {
      field.value = items[i];
      if (values[node.id] !== items[i]) changed(node, items[i]);
    });
    box.appendChild(field);
    box.appendChild(button);
    return box;
  }

  // A number field, as egui's drag value, moving a step a point, in a text
  // field's look.
  function drawNumber(node, p) {
    needNumber(p, 'value');
    var range = bounds(node, p), by = step(p);
    return dragValue({ node: node.id, value: values[node.id], range: range, speed: by, well: true, enabled: flag(p, 'enabled', true), set: function (v) { if (v !== values[node.id]) changed(node, v); return v; } });
  }

  // A slider, as egui's: a rail a slider's width with its handle, and a
  // drag value beside it moving as far a point as the handle does, both
  // setting the value within its range and on its steps.
  function drawSlider(node, p) {
    needNumber(p, 'value');
    var range = bounds(node, p), by = step(p);
    var enabled = flag(p, 'enabled', true);
    var box = el('div', 'row slider' + (enabled ? '' : ' off'));
    var bar = el('input');
    bar.type = 'range';
    bar.min = range[0];
    bar.max = range[1];
    bar.step = by;
    bar.value = values[node.id];
    bar.disabled = !enabled;
    bar.dataset.node = node.id + '-input';
    var travel = cssPoints('--slider-w') - 2 * cssPoints('--interact-h') / 2.5;
    var onStep = function (v) {
      var s = range[0] + Math.round((v - range[0]) / by) * by;
      return Math.min(Math.max(Number(s.toFixed(12)), range[0]), range[1]);
    };
    var field = dragValue({
      node: node.id + '-value', value: values[node.id], range: range, speed: (range[1] - range[0]) / (travel > 0 ? travel : 1), enabled: enabled,
      set: function (v) { var kept = onStep(v); bar.value = kept; if (kept !== values[node.id]) changed(node, kept); return kept; }
    });
    bar.oninput = function () {
      var v = bar.valueAsNumber;
      field.setValue(v);
      if (v !== values[node.id]) changed(node, v);
    };
    box.appendChild(bar);
    box.appendChild(field);
    return box;
  }

  // A drop-down, as egui's combo box.
  function drawCombo(node, p) {
    if (flag(p, 'editable', false)) return drawEditableCombo(node, p);
    var items = needStrings(p, 'items');
    var c = chosen(node, items);
    var button = comboButton(c === null ? '' : items[c], flag(p, 'enabled', true), false);
    button.dataset.node = node.id;
    attachChoices(button, node, items, function () { return chosen(node, items); }, function (i) {
      if (values[node.id] !== i) changed(node, i);
      redraw(node);
    });
    return button;
  }

  // Option buttons, as egui's radio buttons, one under another.
  function drawRadio(node, p) {
    var items = needStrings(p, 'items');
    var c = chosen(node, items);
    var on = flag(p, 'enabled', true);
    var box = el('div', 'col radio');
    box.setAttribute('role', 'radiogroup');
    items.forEach(function (text, i) {
      var label = el('label', 'check' + (on ? '' : ' off'));
      var button = el('input');
      button.type = 'radio';
      button.name = 'radio-' + node.id;
      button.checked = c === i;
      button.disabled = !on;
      button.dataset.node = node.id + '-' + i;
      button.onchange = function () { if (button.checked) changed(node, i); };
      label.appendChild(button);
      label.appendChild(el('span', 'label', text));
      box.appendChild(label);
    });
    return box;
  }

  // The window's own widgets of a kind, the root or its children, which is
  // where a menu bar and a status bar stand.
  function ownOf(kind) {
    if (tree.kind === kind) return [tree];
    return (tree.children || []).filter(function (c) { return c.kind === kind; });
  }
  // Whether render draws node `id` rather than draw: a menu bar, a status
  // bar, a button bar, a context menu, or an entry of a menu.
  function chrome(id) {
    var walk = function (n, inMenu) {
      if (n.id === id) return inMenu || ['MenuBar', 'StatusBar', 'ButtonBar', 'ContextMenu'].indexOf(n.kind) >= 0;
      var deeper = inMenu || ['MenuBar', 'Menu', 'ContextMenu'].indexOf(n.kind) >= 0;
      var kids = n.children || [];
      for (var i = 0; i < kids.length; i++) { var found = walk(kids[i], deeper); if (found !== null) return found; }
      return null;
    };
    return walk(tree, false) === true;
  }
  // The element `make` draws for a menu's entry, or the entry's problem in
  // its place.
  function entryOf(node, make) {
    var e;
    try { e = make(); } catch (err) {
      var label = node.kind + ' ' + (node.name || node.id) + ': ' + err.message;
      post({ t: 'error', text: label });
      e = el('div', 'error', label);
    }
    e.dataset.node = node.id;
    hook(e, node);
    return e;
  }
  // A shortcut as a menu shows it: as written, with Ctrl shown as Cmd on
  // macOS, where it stands for the Command key.
  function shownShortcut(written) { return mac ? written.replace(/^Ctrl\+/, 'Cmd+') : written; }

  function drawMenuBar(bar) {
    var box = el('div', 'menubar');
    box.dataset.node = bar.id;
    hook(box, bar);
    (bar.children || []).forEach(function (menu) {
      box.appendChild(entryOf(menu, function () {
        if (menu.kind !== 'Menu') throw new Error('a MenuBar contains only Menu widgets, and this one contains a ' + menu.kind);
        var title = el('button', 'btn frame plain title', need(props(menu), 'title', 'string'));
        title.onclick = function () {
          if (openMenus.length && openMenus[0].node === menu) { closeMenus(0); return; }
          var r = title.getBoundingClientRect();
          openMenu(menu, r.left, r.bottom, 0, title);
        };
        // Once one of the bar's menus is open, pointing at another title
        // opens that menu in its place.
        title.onmouseenter = function () {
          var first = openMenus[0];
          if (first && first.opener && first.opener.parentNode === box && first.node !== menu) {
            var r = title.getBoundingClientRect();
            openMenu(menu, r.left, r.bottom, 0, title);
          }
        };
        return title;
      }));
    });
    return box;
  }

  function drawStatusBar(bar) {
    var box = el('div', 'statusbar');
    box.dataset.node = bar.id;
    hook(box, bar);
    (bar.children || []).forEach(function (c) { box.appendChild(draw(c)); });
    return box;
  }

  // A button bar: its buttons at its right in the order the system gives
  // them (pwrsLayout.order), each a dialog's button wide at least and its
  // gap apart, on the strip Windows draws under a dialog's buttons; a child
  // that is not a button shows its problem in its place. A button whose
  // role cannot be read stays in its place among the others and shows its
  // problem. The buttons stand left to right, so Tab moves through them in
  // the order shown.
  function drawButtonBar(bar) {
    var box = el('div', 'buttonbar' + (windowsOrder ? ' strip' : ''));
    box.dataset.node = bar.id;
    hook(box, bar);
    var kids = bar.children || [];
    var roles = kids.map(function (c) {
      try { return pwrsLayout.role(c); } catch (err) { return 'other'; }
    });
    pwrsLayout.order(roles, windowsOrder).forEach(function (i) {
      var c = kids[i];
      if (c.kind !== 'Button') {
        var shown = problemOf(c, 'a ButtonBar contains only Button widgets, and this one contains a ' + c.kind);
        shown.dataset.node = c.id;
        box.appendChild(shown);
      } else {
        box.appendChild(draw(c));
      }
    });
    return box;
  }

  // Closes the menus open at level `from` and deeper.
  function closeMenus(from) {
    openMenus.splice(from).forEach(function (m) {
      m.element.remove();
      if (m.opener) m.opener.classList.remove('open');
    });
  }
  // Opens the entries of `menu`, a Menu or a ContextMenu, at (x, y) or as
  // near it as fits inside the window, as the menu at `level`, in place of
  // any open there or deeper.
  function openMenu(menu, x, y, level, opener) {
    closeMenus(level);
    var box = el('div', 'menu');
    if (menu.kind === 'ContextMenu') {
      box.dataset.node = menu.id;
      hook(box, menu);
    }
    (menu.children || []).forEach(function (entry) { box.appendChild(drawEntry(entry, level)); });
    popups.appendChild(box);
    var r = box.getBoundingClientRect();
    box.style.left = Math.max(0, Math.min(x, window.innerWidth - r.width)) + 'px';
    box.style.top = Math.max(0, Math.min(y, window.innerHeight - r.height)) + 'px';
    if (opener) opener.classList.add('open');
    openMenus.push({ element: box, node: menu, opener: opener });
  }
  // One entry of the open menu at `level`: a MenuItem, its shortcut shown
  // beside its text, raises Click and closes every menu; a Menu opens its
  // entries beside it.
  function drawEntry(entry, level) {
    return entryOf(entry, function () {
      var p = props(entry);
      if (entry.kind === 'Separator') { var line = el('div', 'separator'); line.setAttribute('role', 'separator'); return line; }
      if (entry.kind === 'Menu') {
        var sub = el('button', 'btn frame plain entry');
        sub.appendChild(el('span', '', need(p, 'title', 'string')));
        sub.appendChild(el('span', '', '⏵'));
        var open = function () {
          var shown = openMenus[level + 1];
          if (shown && shown.node === entry) return;
          var r = sub.getBoundingClientRect();
          openMenu(entry, r.right + cssPoints('--menu-gap'), r.top, level + 1, sub);
        };
        sub.onclick = open;
        sub.onmouseenter = open;
        return sub;
      }
      if (entry.kind === 'MenuItem') {
        var item = el('button', 'btn frame plain entry');
        item.appendChild(el('span', '', need(p, 'text', 'string')));
        if (p.shortcut !== undefined) item.appendChild(el('span', 'shortcut', shownShortcut(need(p, 'shortcut', 'string'))));
        item.disabled = !flag(p, 'enabled', true);
        item.onclick = function () { closeMenus(0); emit(entry, 'Click'); };
        item.onmouseenter = function () { closeMenus(level + 1); };
        return item;
      }
      throw new Error('a menu contains only MenuItem, Menu and Separator widgets, and this one contains a ' + entry.kind);
    });
  }
  // The innermost widget drawn at or around `element` whose flag `key` is
  // true, or null.
  function flaggedAt(element, key) {
    for (var e = element; e && e !== document.documentElement; e = e.parentElement) {
      if (!e.dataset || !e.dataset.node) continue;
      var n = find(tree, parseInt(e.dataset.node, 10));
      if (n && props(n)[key] === true) return n;
    }
    return null;
  }
  // The widget a drop at `element` lands on: the innermost taking drops,
  // or else the window's root when it takes them.
  function dropTarget(element) {
    return flaggedAt(element, 'drop') || (props(tree).drop === true ? tree : null);
  }

  // The shortcut a key press makes, { ctrl, shift, alt, key } with the key
  // as the protocol names it, or null for a press no shortcut names. A key
  // is the shortcut key its character is: a letter, a digit, + - and = as
  // Plus, Minus and Equals, or a named key. Where its character is none of
  // those, as with Shift and 1 typing !, Option down on macOS or a layout
  // of another script, it is the key at its place on the keyboard, as egui
  // reads it.
  var symbolKeys = { '+': 'Plus', '-': 'Minus', '=': 'Equals' };
  var placeKeys = { Minus: 'Minus', Equal: 'Equals', NumpadAdd: 'Plus', NumpadSubtract: 'Minus' };
  function pressedKeys(e) {
    if (mac ? e.ctrlKey : e.metaKey) return null;
    var key = null;
    if (e.key === ' ') key = 'Space';
    else if (Object.prototype.hasOwnProperty.call(symbolKeys, e.key)) key = symbolKeys[e.key];
    else if (shortcutKeys.indexOf(e.key) >= 0 || /^F([1-9]|1[0-9]|2[0-4])$/.test(e.key)) key = e.key;
    else if (/^[A-Za-z0-9]$/.test(e.key)) key = e.key.toUpperCase();
    else if (e.key === 'Dead' || e.key === 'Unidentified' || e.key.length === 1) {
      var place = /^(?:Key([A-Z])|Digit([0-9])|Numpad([0-9]))$/.exec(e.code);
      if (place) key = place[1] || place[2] || place[3];
      else if (Object.prototype.hasOwnProperty.call(placeKeys, e.code)) key = placeKeys[e.code];
    }
    if (key === null) return null;
    return { ctrl: mac ? e.metaKey : e.ctrlKey, shift: e.shiftKey, alt: e.altKey, key: key };
  }
  // Keys as a shortcut is written: Ctrl, Shift and Alt in that order, then
  // the key.
  function writtenOf(k) { return (k.ctrl ? 'Ctrl+' : '') + (k.shift ? 'Shift+' : '') + (k.alt ? 'Alt+' : '') + k.key; }
  // Whether a focused text field keeps these keys from the window's and its
  // menus' shortcuts: a key without Ctrl or Alt other than F1 to F24, or
  // Ctrl without Alt and one of the keys the protocol lists.
  function fieldKeeps(k) {
    if (!k.ctrl && !k.alt) return !/^F[0-9]+$/.test(k.key);
    if (k.ctrl && !k.alt) return fieldCtrlKeys.indexOf(k.key) >= 0;
    return false;
  }
  function inField() {
    var a = document.activeElement;
    return !!a && (a.tagName === 'TEXTAREA' || (a.tagName === 'INPUT' && textTypes.indexOf(a.type) >= 0) || a.isContentEditable === true);
  }
  // Whether these keys are Alt+F8, which shows a password field's text
  // while it offers its peek button (proto Shortcut::is_peek).
  function isPeek(k) { return k.alt && !k.ctrl && !k.shift && k.key === 'F8'; }
  // Whether the widget with the focus is a password field, which keeps
  // Alt+F8 from the window's and its menus' shortcuts.
  function inPassword() {
    var id = focusedNode(), n = id === null ? null : find(tree, id);
    return !!n && n.kind === 'TextBox' && props(n).password === true;
  }
  // What a shortcut acts on: the first enabled menu item with it, which it
  // clicks, or else the window's own, which raises Shortcut on the root
  // with the shortcut; null for neither.
  function shortcutTarget(written) {
    var items = [];
    (function walk(n) { if (n.kind === 'MenuItem') items.push(n); (n.children || []).forEach(walk); })(tree);
    for (var i = 0; i < items.length; i++) {
      var p = props(items[i]);
      if (p.enabled !== false && p.shortcut === written) return { node: items[i], kind: 'Click', value: null };
    }
    var own = props(tree).shortcuts;
    if (Array.isArray(own) && own.indexOf(written) >= 0) return { node: tree, kind: 'Shortcut', value: written };
    return null;
  }

  // The value a test gives an input, checked as the input checks a
  // person's.
  function takes(node, v) {
    var p = props(node);
    if (node.kind === 'CheckBox') {
      if (typeof v !== 'boolean') throw new Error(JSON.stringify(v) + ' is not true or false');
    } else if (node.kind === 'TextBox' || (node.kind === 'ComboBox' && p.editable === true)) {
      if (typeof v !== 'string') throw new Error(JSON.stringify(v) + ' is not text');
    } else if (node.kind === 'ColorPicker') {
      if (typeof v !== 'string' || !colorForm.test(v.toLowerCase())) throw new Error(JSON.stringify(v) + ' is not a color written #rrggbb');
      v = v.toLowerCase();
    } else if (node.kind === 'NumberBox' || node.kind === 'Slider') {
      if (typeof v !== 'number' || !isFinite(v)) throw new Error(JSON.stringify(v) + ' is not a number');
      var range = bounds(node, p);
      if (v < range[0] || v > range[1]) throw new Error(v + ' is outside the range ' + range[0] + ' to ' + range[1]);
    } else if (node.kind === 'DatePicker') {
      if (typeof v !== 'string' || !minuteForm.test(v)) throw new Error(JSON.stringify(v) + ' is not a date written YYYY-MM-DDTHH:MM');
    } else if (node.kind === 'FilePicker') {
      if (!Array.isArray(v) || !v.length || v.some(function (x) { return typeof x !== 'string'; })) throw new Error(JSON.stringify(v) + ' is not a list of paths');
    } else if (node.kind === 'Tabs') {
      var tabs = (node.children || []).length;
      if (!Number.isInteger(v) || v < 0) throw new Error(JSON.stringify(v) + ' is not a tab number');
      if (v >= tabs) throw new Error('tab ' + v + ' is beyond the ' + tabs + ' tabs');
    } else {
      var count = needStrings(p, 'items').length;
      if (!Number.isInteger(v) || v < 0) throw new Error(JSON.stringify(v) + ' is not a row number');
      if (v >= count) throw new Error('row ' + v + ' is beyond the ' + count + ' items');
    }
    return v;
  }

  function remove(node, id) {
    var at = (node.children || []).findIndex(function (c) { return c.id === id; });
    if (at >= 0) { node.children.splice(at, 1); return true; }
    return (node.children || []).some(function (c) { return remove(c, id); });
  }

  // Applies one patch; returns how to show it: { node } to draw a node
  // again, { node, added } to add a log's new lines, or null to draw the
  // whole tree again. A change to a tree's items draws the tree again.
  function applyOp(op) {
    if (op.op === 'set') {
      var n = find(tree, op.node);
      if (!n) throw new Error('no node ' + op.node);
      Object.assign(n.props, op.props);
      seed(n, Object.keys(op.props));
      return { node: n.kind === 'TreeItem' ? holderOf(n.id) || n : n };
    }
    if (op.op === 'insert') {
      var under = find(tree, op.parent);
      if (!under) throw new Error('no node ' + op.parent);
      under.children.splice(Math.min(op.index, under.children.length), 0, op.node);
      seed(op.node);
      var grown = under.kind === 'TreeView' ? under : under.kind === 'TreeItem' ? holderOf(under.id) : null;
      return grown ? { node: grown } : null;
    }
    if (op.op === 'remove') {
      var holder = holderOf(op.node);
      if (!remove(tree, op.node)) throw new Error('no node ' + op.node + ' to remove');
      return holder ? { node: holder } : null;
    }
    if (op.op === 'append') {
      var a = find(tree, op.node);
      if (!a) throw new Error('no node ' + op.node);
      if (a.props[op.prop] === undefined) a.props[op.prop] = [];
      var list = a.props[op.prop];
      if (!Array.isArray(list)) throw new Error('node ' + op.node + "'s " + op.prop + ' is not a list to add to');
      op.values.forEach(function (v) { list.push(v); });
      seed(a, [op.prop]);
      return { node: a, added: op.values };
    }
    if (op.op === 'replace') { var old = find(tree, op.node.id); if (!old) throw new Error('no node ' + op.node.id + ' to replace'); Object.keys(old).forEach(function (k) { delete old[k]; }); Object.assign(old, op.node); seed(old); return null; }
    if (op.op === 'removeRows') {
      var held = find(tree, op.node), t = tables[op.node];
      if (!held || !t) throw new Error('no table ' + op.node);
      var gone = new Set(op.rows), moved = [], kept = [];
      t.rows.forEach(function (row, i) { if (gone.has(i)) moved.push(null); else { moved.push(kept.length); kept.push(row); } });
      t.rows = kept;
      var to = function (r) { return r < moved.length ? moved[r] : null; };
      t.selected = new Set(sorted(t.selected).map(to).filter(function (r) { return r !== null; }));
      t.anchor = t.anchor === null || t.anchor === undefined ? null : to(t.anchor);
      t.dirty = true;
      return { node: held };
    }
    if (op.op === 'setItem' || op.op === 'addItems' || op.op === 'removeItems') {
      changeItems(op);
      return { quiet: true };
    }
    throw new Error('no patch of kind ' + op.op);
  }

  // The box each named node is drawn in, by node id, in the page's points:
  // a node not drawn, as one on a tab not shown or a menu closed, has none.
  function boundsOf(node, out) {
    if (node.name) {
      var drawn = document.querySelector('[data-node="' + node.id + '"]');
      if (drawn && drawn.getClientRects().length) {
        var r = drawn.getBoundingClientRect();
        out[node.id] = { x: r.left, y: r.top, width: r.width, height: r.height };
      }
    }
    (node.children || []).forEach(function (c) { boundsOf(c, out); });
    return out;
  }

  // Reports the window open: after the first two frames and, where fonts
  // are still loading then, their arrival and two frames more, so a window
  // whose text is drawn in a font loaded from the helper reports open with
  // its text laid out in it.
  function reportOpened() {
    requestAnimationFrame(function () {
      requestAnimationFrame(function () {
        if (document.fonts.status !== 'loading') { post({ t: 'opened' }); return; }
        document.fonts.ready.then(function () {
          requestAnimationFrame(function () { requestAnimationFrame(function () { post({ t: 'opened' }); }); });
        });
      });
    });
  }

  // The window's own stylesheet, the CSS its root's `stylesheet` gives, as
  // the last of the page's styles, so its rules come after the runtime's.
  // Each open replaces it, so a page loaded again takes it once.
  function applyStylesheet(css) {
    var old = document.getElementById('pwrs-stylesheet');
    if (old) old.remove();
    if (css === undefined || css === null) return;
    if (typeof css !== 'string') { post({ t: 'error', text: 'open: the window\'s stylesheet is ' + JSON.stringify(css) + ', not CSS text, so the runtime\'s look stands alone' }); return; }
    var sheet = document.createElement('style');
    sheet.id = 'pwrs-stylesheet';
    sheet.textContent = css;
    document.head.appendChild(sheet);
  }

  window.pwrsRuntime = {
    // Draws the window `o` gives: its tree, its stylesheet, its inputs'
    // state, the system's button order, its theme when it has one of its
    // own, the protocol's key lists and the engine's name. A window hidden
    // until it is measured (`fitting`) reports open once the helper has
    // shown it.
    open: function (o) {
      tree = o.tree;
      applyStylesheet(props(tree).stylesheet);
      windowsOrder = o.windowsOrder === true;
      dialogWindow = o.dialog === true;
      if (o.theme === 'light' || o.theme === 'dark') document.documentElement.dataset.theme = o.theme;
      else delete document.documentElement.dataset.theme;
      if (o.theme !== null && o.theme !== undefined && o.theme !== 'light' && o.theme !== 'dark') post({ t: 'error', text: 'open: the theme ' + JSON.stringify(o.theme) + ' is neither light nor dark, so the window follows the system' });
      if (typeof o.engine === 'string') engineName = o.engine;
      if (!Array.isArray(o.keys) || !Array.isArray(o.fieldCtrlKeys)) post({ t: 'error', text: 'open: the window came without the shortcut key lists, so no shortcut acts' });
      shortcutKeys = Array.isArray(o.keys) ? o.keys : [];
      fieldCtrlKeys = Array.isArray(o.fieldCtrlKeys) ? o.fieldCtrlKeys : [];
      viewNumbers = o.view || null;
      seed(tree);
      restore(o.state);
      render();
      // Laying the page out starts the loads of the fonts its text needs.
      document.body.getBoundingClientRect();
      if (o.fitting !== true) reportOpened();
    },
    // Measures the window's widgets as a window fitting its content takes
    // them: laid out at most `o.most[0]` points across, each at its own
    // size, once the fonts their text needs and the pictures the session
    // gave the window have loaded or failed, an https picture measured as
    // the spinner it stands as while loading. Posts the size, in points
    // rounded up to a whole physical pixel, which the helper sizes the
    // window to before it shows it, or why it could not be measured, which
    // has the helper show it at the most it may take.
    measure: function (o) {
      var html = document.documentElement;
      var done = function () {
        html.classList.remove('measuring');
        html.style.removeProperty('--fit-most-w');
        fit();
      };
      var failed = function (err) {
        done();
        post({ t: 'fit', size: null, error: 'the window could not be measured: ' + (err && err.message ? err.message : String(err)) });
      };
      try {
        var most = o && Array.isArray(o.most) && typeof o.most[0] === 'number' ? o.most[0] : null;
        if (most !== null) html.style.setProperty('--fit-most-w', most + 'px');
        html.classList.add('measuring');
        document.body.getBoundingClientRect();
        var served = Array.prototype.filter.call(root.querySelectorAll('img'), function (img) { return !img.classList.contains('web') && !img.complete; });
        var pictures = served.map(function (img) {
          return new Promise(function (settled) {
            img.addEventListener('load', settled, { once: true });
            img.addEventListener('error', settled, { once: true });
          });
        });
        Promise.all([document.fonts.ready].concat(pictures)).then(function () {
          try {
            var r = root.getBoundingClientRect();
            var ratio = window.devicePixelRatio || 1;
            var up = function (v) { return Math.ceil(v * ratio - 0.001) / ratio; };
            done();
            post({ t: 'fit', size: [up(r.width), up(r.height)] });
          } catch (err) { failed(err); }
        }, failed);
      } catch (err) { failed(err); }
    },
    // The helper has shown a window that was hidden until it was measured.
    shown: function () { reportOpened(); },
    patch: function (ops) {
      var whole = false, shown = [];
      ops.forEach(function (op) {
        try { var r = applyOp(op); if (r) shown.push(r); else whole = true; } catch (err) { post({ t: 'error', text: 'Patch: ' + err.message }); }
      });
      reportAll();
      forgetGone();
      if (whole) { render(); return; }
      shown.forEach(function (r) {
        if (r.quiet) return;
        if (r.added && r.node.kind === 'LogView') addLines(r.node, r.added); else redraw(r.node);
      });
    },
    // What GpuSurface `node` draws, as the session gave it: built on the
    // page's device and stored once WebGPU takes every pass and draw, which
    // replies to the helper's request `ask`; a page loaded again is given
    // its surfaces with no request.
    surface: function (node, given, ask) {
      give(surfaceOf(node), given, ask === undefined ? null : ask);
    },
    // GpuSurface `node`'s camera as the session set it: its pose, its
    // projection and the gestures its controls bind, each with the move it
    // makes, in the order they are matched; null takes it away, so the
    // frame uniform's view and projection are identity matrices again.
    camera: function (node, spec) {
      var s = surfaceOf(node);
      s.camera = spec;
      if (!spec) s.held = null;
      s.dirty = true;
      schedule();
    },
    // Canvas `node`'s view as the helper stored it, for a page loaded again,
    // or null to take it away with any move and drag of it.
    view: function (node, spec) {
      var c = canvasState(node);
      c.view = spec ? takenView(spec) : null;
      c.motion = null;
      dropHeld(c);
      c.fitPending = null;
      c.reportedView = null;
      c.dirty = true;
      takeFocus(c);
      schedule();
    },
    // A change the session made to canvas `node`'s view, which the helper
    // checked as egui checks it: made from the view drawn now, or the
    // default view; once the canvas is drawn its fit is made at once and
    // the view moves to it over its ease, and before that the change is
    // taken at once and its fit made at the first frame. Replies to the
    // helper's `ask` with the view as then drawn.
    setView: function (node, change, ask) {
      var c = canvasState(node), now = seconds();
      var size = c.place ? [c.place.across / c.place.ppp, c.place.down / c.place.ppp] : null;
      if (size) settleView(c, now, size);
      var base = c.view || { center: null, zoom: 1, minZoom: 2.2250738585072014e-308, maxZoom: Infinity, bindings: change.bindings };
      var part = function (key) { var v = change[key]; return v === undefined ? base[key] : v === 'Infinity' ? Infinity : v; };
      var made = { center: change.center === undefined ? base.center : change.center, zoom: part('zoom'), minZoom: part('minZoom'), maxZoom: part('maxZoom'), bindings: change.bindings || base.bindings };
      if (size && c.frame) {
        var to = change.fit ? fittedView(c, made, change.fit, size) : made;
        if (change.ease) easeView(c, to, change.ease, now, size); else c.view = to;
      } else {
        c.view = made;
        c.fitPending = change.fit || null;
      }
      c.dirty = true;
      takeFocus(c);
      schedule();
      post({ t: 'gpu', request: ask, ok: answeredView(c, now, size) });
    },
    // The helper stores the copy at `name` at `version` now: the surfaces
    // drawing from it take it again and draw again.
    copied: function (name, version) {
      var c = copies[name];
      if (!c || c.version >= version) return;
      var users = Object.keys(surfaces).filter(function (id) { var plan = surfaces[id].plan; return plan && plan.names.indexOf(name) >= 0; });
      if (!users.length) return;
      var r = Object.assign({}, c.resource, { version: version });
      var taken = gpu.mode === 'webgl2' ? takeBytes(r) : gpu.device ? take(gpu.device, r) : null;
      if (!taken) return;
      taken.then(function (kept) {
        users.forEach(function (id) {
          var s = surfaces[id];
          if (!s) return;
          if (s.gl && !s.gl.isContextLost()) glRefresh(s, kept);
          s.dirty = true;
        });
        schedule();
      }, function (err) {
        users.forEach(function (id) { if (surfaces[id]) surfaceProblem(surfaces[id], err.message); });
      });
    },
    // The texels of a frame of GpuSurface `node` drawn now, rows of red,
    // green, blue and alpha bytes from the top, replying to the helper's
    // request `ask`; a surface not drawn, as one on a tab not shown or
    // given no room, has none. A WebGL 2 context the engine has taken away
    // is waited for, and the surface built in it again, before the frame.
    save: function (node, ask) {
      var s = surfaces[node];
      var refused = function (id, text) { post({ t: 'gpu', request: ask, error: { id: id, text: text } }); };
      var shown = find(tree, node);
      var notDrawn = 'GpuSurface ' + (shown && shown.name ? shown.name : node) + ' is not drawn, as a surface on a tab not shown or given no room in its window is not';
      if (!s || !s.canvas || !s.canvas.isConnected) { refused('GuiGpuNotDrawn', notDrawn); return; }
      gpuMode().then(function (mode) {
        if (mode !== 'webgl2') return gpuDevice().then(saveGpu);
        glOf(s);
        return glReturned(s).then(function () { return s.builds; }).then(function () {
          var read = {};
          var drawn = frameGl(s, read, null);
          if (!drawn) refused('GuiGpuNotDrawn', notDrawn);
          else post({ t: 'gpu', request: ask, ok: { width: drawn.across, height: drawn.down, rgba: base64Of(read.pixels) } });
          return null;
        });
      }).then(null, function (err) { refused(err.id || 'GuiGpuSave', 'the surface could not be read: ' + err.message); });
      // The WebGPU frame read back: drawn now, its canvas texture copied to
      // a buffer of rows WebGPU aligns, and the rows packed.
      function saveGpu(device) {
        var at = placeOf(s);
        if (at.across < 1 || at.down < 1) { refused('GuiGpuNotDrawn', notDrawn); return null; }
        var rowBytes = Math.ceil(at.across * 4 / 256) * 256;
        var buffer = device.createBuffer({ size: rowBytes * at.down, usage: GPUBufferUsage.COPY_DST | GPUBufferUsage.MAP_READ });
        if (!frame(s, { buffer: buffer, rowBytes: rowBytes })) {
          buffer.destroy();
          refused('GuiGpuNotDrawn', notDrawn);
          return null;
        }
        return buffer.mapAsync(GPUMapMode.READ).then(function () {
          var mapped = new Uint8Array(buffer.getMappedRange());
          var rowTexels = at.across * 4;
          var rgba = new Uint8Array(rowTexels * at.down);
          for (var y = 0; y < at.down; y++) rgba.set(mapped.subarray(y * rowBytes, y * rowBytes + rowTexels), y * rowTexels);
          buffer.unmap();
          buffer.destroy();
          post({ t: 'gpu', request: ask, ok: { width: at.across, height: at.down, rgba: base64Of(rgba) } });
        });
      }
    },
    // The helper's reply to the page's request `ask` for a draw's GLSL: its
    // stages, or the error the draw is refused with.
    glsl: function (ask, answer) {
      var waiting = glslAsks[ask];
      if (!waiting) return;
      delete glslAsks[ask];
      if (answer && answer.error) waiting.reject(gpuError(answer.error.id, answer.error.text));
      else waiting.resolve(answer);
    },
    // Rows for table `node` from row `start`; `added` for rows Add-GuiItem
    // added, which a table showing its last row follows (fit).
    rows: function (node, start, rows, added) {
      var t = tables[node] || (tables[node] = newTable());
      t.rows = t.rows.slice(0, Math.min(start, t.rows.length)).concat(rows);
      t.dirty = true;
      if (added === true) t.added = true;
      var n = find(tree, node);
      if (n) redraw(n); else render();
    },
    test: function (call) {
      try {
        // A shortcut names no widget: it acts as a person's press does.
        if (call.kind === 'Shortcut') {
          var hit = shortcutTarget(call.value);
          if (!hit) throw new Error('this window has no enabled menu item and no shortcut of its own for ' + call.value);
          post({ t: 'test', request: call.request, ok: hit.value, node: hit.node.id });
          closeMenus(0);
          emit(hit.node, hit.kind, hit.value);
          return;
        }
        var node = findNamed(tree, call.name);
        if (!node) throw new Error('this window has no widget named ' + call.name);
        // A disabled widget takes no click, change, Enter or key from a
        // person, so it takes none from a test.
        if (['Click', 'Change', 'Submit', 'Key'].indexOf(call.kind) >= 0 && !flag(props(node), 'enabled', true)) {
          throw new Error('the ' + (node.kind === 'MenuItem' ? 'menu item' : node.kind) + ' is disabled, so it raises nothing');
        }
        var value = null;
        if ((node.kind === 'Button' || (node.kind === 'Text' && props(node).style === 'link')) && call.kind === 'Click') {
          value = null;
        } else if (node.kind === 'Markdown' && call.kind === 'Click') {
          var doc = props(node).document || {};
          var links = Array.isArray(doc.links) ? doc.links : [];
          if (typeof call.value !== 'string') throw new Error(JSON.stringify(call.value) + " is not a link's address");
          if (links.indexOf(call.value) < 0) throw new Error('the document has no link to ' + call.value + '; ' + (links.length ? 'its links are ' + links.join(', ') : 'it has none'));
          if (call.value.charAt(0) === '#') {
            // A link to a heading scrolls there and raises nothing, as a
            // person's click does.
            post({ t: 'test', request: call.request, ok: null, node: node.id });
            var shownDoc = document.querySelector('[data-node="' + node.id + '"]');
            if (shownDoc) scrollToHeading(shownDoc, call.value.slice(1));
            return;
          }
          value = call.value;
        } else if (node.kind === 'MenuItem' && call.kind === 'Click') {
          value = null;
        } else if ((node.kind === 'ListBox' || node.kind === 'Table') && call.kind === 'Select') {
          var rows = rowsOf(call.value);
          var held = node.kind === 'ListBox' ? lists[node.id] : tables[node.id];
          held.selected = new Set(rows);
          // The keyboard goes to the last row selected, as a click there takes it.
          if (rows.length) held.current = held.anchor = Math.max.apply(null, rows);
          value = rows;
        } else if ((node.kind === 'ListBox' || node.kind === 'Table') && call.kind === 'Activate') {
          var r = rowsOf(call.value);
          if (r.length !== 1) throw new Error(JSON.stringify(call.value) + ' is not a row number');
          if (node.kind === 'ListBox') lists[node.id].selected = new Set(r); else tables[node.id].selected = new Set(r);
          value = r[0];
        } else if (node.kind === 'TreeView' && (call.kind === 'Select' || call.kind === 'Activate' || call.kind === 'Expand')) {
          // A tree's items are named by their rows as shown, from 0.
          checkItems(node.children);
          var tv = trees[node.id];
          var shown = shownItems(node.children, 0, []);
          var rowItem = function (n) { if (n >= shown.length) throw new Error('row ' + n + " is beyond the tree's " + shown.length + ' rows as shown'); return shown[n].item; };
          var picked = rowsOf(call.value);
          if (call.kind === 'Select') {
            if (!flag(props(node), 'multiple', false) && picked.length > 1) throw new Error('the tree selects one row, and ' + picked.length + ' were given');
            tv.selected = new Set(picked.map(function (n) { return rowItem(n).id; }));
            tv.anchor = picked.length ? rowItem(picked[picked.length - 1]).id : null;
            value = selectionOf(tv, node);
          } else {
            if (picked.length !== 1) throw new Error(JSON.stringify(call.value) + ' is not a row number');
            var it = rowItem(picked[0]);
            if (call.kind === 'Activate') {
              tv.selected = new Set([it.id]);
              tv.anchor = it.id;
            } else if (!opens(it)) {
              throw new Error('row ' + picked[0] + ' is a leaf, with nothing to open');
            } else if (!lazy(it)) {
              // Opening an item with its children raises nothing, as a
              // person's opening it does.
              openItems.add(it.id);
              post({ t: 'test', request: call.request, ok: it.id, node: node.id });
              report(node.id);
              redraw(node);
              return;
            } else {
              if (expanding.has(it.id)) throw new Error('row ' + picked[0] + ' is waiting for its children already');
              expanding.add(it.id);
              openItems.add(it.id);
            }
            value = it.id;
          }
        } else if (node.kind === 'TextBox' && call.kind === 'Peek') {
          // A test holds or lets go of a password field's peek button, as a
          // person pressing it does; it raises nothing.
          syncPeeks();
          var rule = peeks[node.id];
          if (!rule) throw new Error(call.name + ' is no password field, so it has no peek button');
          if (typeof call.value !== 'boolean') throw new Error(JSON.stringify(call.value) + ' is not true or false');
          if (call.value && !rule.offered()) throw new Error(call.name + ' offers no peek button now: a password field offers it once it has the focus and is typed into, and not again after the focus leaves until it is cleared');
          if (call.value) peekHeld[node.id] = true; else delete peekHeld[node.id];
          if (peekShows[node.id]) peekShows[node.id]();
          post({ t: 'test', request: call.request, ok: call.value, node: node.id });
          return;
        } else if (call.kind === 'Change' && inputKinds.indexOf(node.kind) >= 0) {
          value = takes(node, call.value);
          values[node.id] = value;
          if (peeks[node.id]) peeks[node.id].update(focusedNode() === node.id, value);
        } else if (call.kind === 'Submit' && node.kind === 'TextBox' && props(node).multiline !== true) {
          value = takes(node, call.value);
          values[node.id] = value;
        } else if (node.kind === 'GpuSurface' && ['PointerDown', 'PointerUp', 'PointerMove', 'Wheel'].indexOf(call.kind) >= 0) {
          // A pointer event at fractions of the surface, which takes its
          // place in pixels from where the surface is drawn.
          var asked = { PointerDown: 'pointerDown', PointerUp: 'pointerUp', PointerMove: 'pointerMove', Wheel: 'wheel' }[call.kind];
          if (!flag(props(node), asked, false)) throw new Error(call.name + ' raises no ' + call.kind + "; its command's -On" + call.kind + ' gives it one');
          var drawn = surfaces[node.id];
          var place = drawn && drawn.canvas && drawn.canvas.isConnected ? placeOf(drawn) : null;
          if (!place || place.across < 1 || place.down < 1) throw new Error('GpuSurface ' + call.name + ' has not been drawn, so there is no place to put the pointer on');
          value = pointerTestValue(place, call.kind, call.value);
        } else if (node.kind === 'Canvas') {
          // A canvas event the helper checked, at the middle of where the
          // named item was last drawn, topmost first, or at fractions of
          // the canvas, naming the topmost named item there as a person's
          // pointer would; an item's event where no named item is raises
          // nothing.
          var cv = canvases[node.id], given = call.value;
          if (!cv || !cv.place || !cv.frame || !cv.canvas || !cv.canvas.isConnected) throw new Error('Canvas ' + call.name + ' has not been drawn, so there is no place to put the pointer on');
          var u = given.u, v = given.v;
          if (given.item !== null) {
            var drawnAs = cv.frame.hits.slice().reverse().find(function (h) { return sameItemName(h.name, given.item); });
            if (!drawnAs || !drawnAs.span) throw new Error(call.name + ' drew no item named ' + given.item);
            u = (drawnAs.span[0] + drawnAs.span[2] / 2) / cv.place.across;
            v = (drawnAs.span[1] + drawnAs.span[3] / 2) / cv.place.down;
          }
          var px = u * cv.place.across, py = v * cv.place.down;
          value = { x: px, y: py, u: u, v: v, ppp: cv.place.ppp, button: given.button, shift: given.shift, ctrl: given.ctrl, alt: given.alt };
          if (call.kind === 'Wheel') value.notches = given.notches;
          value.item = itemAt(cv, px, py);
          if (given.itemEvent && value.item === null) {
            post({ t: 'test', request: call.request, ok: null, node: node.id });
            return;
          }
          withScene(cv, value);
        } else {
          throw new Error('a ' + node.kind + ' does not raise ' + call.kind);
        }
        // The reply goes before the event, so a command the event ends,
        // closing the window, finds the request replied to.
        post({ t: 'test', request: call.request, ok: value, node: node.id });
        emit(node, call.kind, value);
        if (node.kind === 'MenuItem') { closeMenus(0); return; }
        if (node.kind === 'GpuSurface' || node.kind === 'Canvas') return;
        report(node.id);
        redraw(node);
      } catch (err) {
        post({ t: 'test', request: call.request, error: err.message });
      }
    },
    // Gives widget `id` the keyboard focus, as Set-GuiFocus requests, on the
    // element focusTarget picks, shown as a focus the keyboard gave; the
    // reply refuses a widget the page has not drawn, as one on a tab not
    // shown, and one none of whose parts takes the focus now, as a Canvas
    // taking no keys.
    focus: function (request, id) {
      try {
        var node = find(tree, id);
        if (!node) throw new Error('the window has no widget with the number ' + id);
        focusTarget(node).focus({ focusVisible: true });
        syncPeeks();
        post({ t: 'focus', request: request });
      } catch (err) {
        post({ t: 'focus', request: request, error: err.message });
      }
    },
    // Files dropped at a place on the page, in its points: the widget there
    // taking drops raises Drop with their paths.
    dropped: function (paths, x, y) {
      var target = dropTarget(document.elementFromPoint(x, y));
      if (target) emit(target, 'Drop', paths);
    },
    // A step of a test's input, numbered `ask`, which the page waits for
    // through its bridge, as a page of the caller's own does
    // (window.pwrs.__input); a step of null stops the waiting for it.
    input: function (ask, step) {
      window.pwrs.__input(ask, step);
    },
    // A file chooser's dialog was answered: its button can be pressed
    // again, and paths chosen become its value, raising Change.
    dialogDone: function (id, paths) {
      browsing.delete(id);
      var n = find(tree, id);
      if (!n) return;
      if (Array.isArray(paths)) changed(n, paths);
      redraw(n);
    },
    snapshot: function (request, node) {
      var state = {};
      Object.keys(lists).forEach(function (id) { if (find(tree, Number(id))) state[id] = sorted(lists[id].selected); });
      Object.keys(tables).forEach(function (id) {
        if (!find(tree, Number(id))) return;
        var t = tables[id];
        refresh(t);
        state[id] = { selected: sorted(t.selected), rows: t.rows.length, shown: t.view.length, filter: t.filter };
      });
      Object.keys(trees).forEach(function (id) {
        var n = find(tree, Number(id));
        if (!n) return;
        var order = treeOrder(n.children, []);
        state[id] = { selected: selectionOf(trees[id], n), open: order.filter(function (i) { return openItems.has(i); }), shown: shownItems(n.children, 0, []).map(function (r) { return r.item.id; }) };
      });
      Object.keys(values).forEach(function (id) { if (find(tree, Number(id))) state[id] = values[id]; });
      syncPeeks();
      var offers = {};
      Object.keys(peeks).forEach(function (id) { if (find(tree, Number(id))) offers[id] = peeks[id].offered(); });
      var answer = { t: 'snapshot', request: request, state: state, bounds: boundsOf(tree, {}), focused: focusedNode(), peeks: offers, copied: lastCopied, viewport: { width: window.innerWidth, height: window.innerHeight } };
      if (node !== null && node !== undefined) {
        var drawn = document.querySelector('[data-node="' + node + '"]');
        if (!drawn) { post({ t: 'snapshot', request: request, error: 'widget ' + node + ' is not drawn in this window' }); return; }
        var r = drawn.getBoundingClientRect();
        answer.rect = { x: r.left, y: r.top, width: r.width, height: r.height };
      }
      post(answer);
    }
  };

  // The node of the widget with the focus: the innermost one drawn at or
  // around the focused element, or null.
  function focusedNode() {
    for (var e = document.activeElement; e && e !== document.documentElement; e = e.parentElement) {
      if (!e.dataset || !e.dataset.node) continue;
      var id = parseInt(e.dataset.node, 10);
      if (find(tree, id)) return id;
    }
    return null;
  }

  // The element of widget `node` that Set-GuiFocus focuses, as the egui
  // renderer picks it: a table's rows, option buttons' option chosen, or
  // the first while none is, and otherwise, of the elements drawn for the
  // widget and its parts, the first in the page that Tab reaches, which is
  // a slider's rail and an editable drop-down's field.
  function focusTarget(node) {
    var own = String(node.id), parts = own + '-';
    var drawn = Array.prototype.filter.call(document.querySelectorAll('[data-node]'), function (e) {
      var d = e.dataset.node;
      return (d === own || d.indexOf(parts) === 0) && e.getClientRects().length > 0;
    });
    if (!drawn.length) throw new Error('the window has not drawn the widget, as on a tab that is not shown');
    var takes = drawn.filter(function (e) { return e.tabIndex >= 0 && !e.disabled; });
    if (!takes.length) {
      throw new Error(node.kind === 'Canvas' ? "the canvas takes no keys, neither for a key handler nor for its view's controls" : "none of the widget's parts takes the focus now");
    }
    var named = function (part) { return takes.filter(function (e) { return e.dataset.node === parts + part; })[0]; };
    if (node.kind === 'Table' && named('rows')) return named('rows');
    if (node.kind === 'RadioGroup') {
      var c = chosen(node, needStrings(props(node), 'items'));
      var picked = named(c === null ? 0 : c);
      if (picked) return picked;
    }
    return takes[0];
  }

  // Keys reach the window's shortcuts before any widget: Escape closes an
  // open popup or menu first, a focused text field keeps the keys it edits
  // with and a password field Alt+F8, and a shortcut that acts takes its
  // press from the widget. Enter
  // and Escape pressed alone go to the helper with the widget that has the
  // focus, which presses the window's default or cancel button unless that
  // widget uses the key itself, and closes a dialog with no cancel button
  // on Escape; they go once the key's own handlers have run, so a field's
  // Submit comes first, as egui raises it before it presses the button.
  document.addEventListener('keydown', function (e) {
    if (e.key === 'Escape' && popup) { var anchor = popup.anchor; closePopup(); anchor.focus(); e.preventDefault(); return; }
    if (e.key === 'Escape' && openMenus.length) { closeMenus(0); e.preventDefault(); return; }
    var keys = pressedKeys(e);
    if (keys && !(inField() && (fieldKeeps(keys) || (isPeek(keys) && inPassword())))) {
      var hit = shortcutTarget(writtenOf(keys));
      if (hit) {
        e.preventDefault();
        e.stopPropagation();
        closeMenus(0);
        emit(hit.node, hit.kind, hit.value);
        return;
      }
    }
    var alone = !e.ctrlKey && !e.shiftKey && !e.altKey && !e.metaKey;
    if ((e.key === 'Enter' || e.key === 'Escape') && alone && !e.repeat && !e.isComposing && !openMenus.length && !popup) {
      var key = e.key, held = focusedNode();
      setTimeout(function () { post({ t: 'role', key: key, node: held }); }, 0);
    }
  }, true);
  // A key the window's shortcuts left, pressed in a widget whose keys is
  // true, goes to the helper, which applies the rule naming the keys each
  // widget uses itself; a press goes once, not again while the key is down.
  document.addEventListener('keydown', function (e) {
    if (e.repeat) return;
    var keys = pressedKeys(e);
    var n = keys ? flaggedAt(document.activeElement, 'keys') : null;
    if (n) post({ t: 'key', node: n.id, keys: writtenOf(keys), typed: inField() });
  });
  // Text dragged onto a widget taking drops is dropped there and raises
  // Drop with the text; files come through the helper with their paths.
  document.addEventListener('dragover', function (e) {
    if (!e.dataTransfer || Array.prototype.indexOf.call(e.dataTransfer.types, 'text/plain') < 0 || !dropTarget(e.target)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  });
  document.addEventListener('drop', function (e) {
    var target = dropTarget(e.target);
    var text = target && e.dataTransfer ? e.dataTransfer.getData('text/plain') : '';
    if (!text) return;
    e.preventDefault();
    emit(target, 'Drop', text);
  });
  // Ctrl+C, Cmd+C on macOS, copies the rows selected of the table whose rows
  // have the focus, as Out-GridView copies its own (pwrsLayout.copied), each
  // row ended as the system ends a line; with none selected it copies
  // nothing. The engine raises its copy event at the page's selection or its
  // body, so the page takes it there; taking beforecopy lets the engine copy
  // with no text of the page selected.
  function rowsToCopy() {
    var at = document.activeElement, named = at && at.dataset ? /^(\d+)-rows$/.exec(at.dataset.node || '') : null;
    var t = named ? tables[named[1]] : null;
    if (!t) return [];
    return t.view.filter(function (r) { return t.selected.has(r); }).map(function (r) { return t.rows[r].map(function (c) { return c.text; }); });
  }
  document.addEventListener('beforecopy', function (e) { if (rowsToCopy().length) e.preventDefault(); });
  document.addEventListener('copy', function (e) {
    var rows = rowsToCopy();
    if (!rows.length) return;
    lastCopied = pwrsLayout.copied(rows, NEWLINE);
    e.clipboardData.setData('text/plain', lastCopied);
    e.preventDefault();
  });
  // The engine's own copy or cut of the text selected is the window's last
  // copy too: a field's selection, but none in a password field, from which
  // the engine copies nothing, or else the page's.
  ['copy', 'cut'].forEach(function (type) {
    document.addEventListener(type, function (e) {
      if (e.defaultPrevented) return;
      var f = document.activeElement, field = f && (f.tagName === 'TEXTAREA' || (f.tagName === 'INPUT' && textTypes.indexOf(f.type) >= 0));
      var text = field ? (f.type === 'password' ? '' : f.value.slice(f.selectionStart, f.selectionEnd)) : String(window.getSelection());
      if (text) lastCopied = text;
    });
  });
  // A press anywhere but on an open menu or a menu bar closes the menus.
  document.addEventListener('pointerdown', function (e) {
    if (popup && !popup.element.contains(e.target) && !popup.anchor.contains(e.target)) closePopup();
    if (!openMenus.length) return;
    for (var t = e.target; t && t.classList; t = t.parentElement) {
      if (t.classList.contains('menu') || t.classList.contains('menubar')) return;
    }
    closeMenus(0);
  }, true);
  window.addEventListener('resize', fit);
  layRows();
  post({ t: 'ready' });
})();
