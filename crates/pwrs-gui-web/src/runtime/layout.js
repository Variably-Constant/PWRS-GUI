// A window's layout rules, as the protocol crate has them for egui:
// which widgets take room where they stand among their siblings (proto
// in_flow), which fill down and share the height their Column leaves
// (proto fills_down), how a Column, Row or Grid places its widgets across
// (proto Align::of), a button's role in the window's keys and the order a
// button bar shows its buttons in (proto button_bar), the keys a list,
// table or tree uses itself (proto rows), and a chart's X as text and a
// date axis's labels (proto chart). Nodes are as the helper sends them:
// kind, props and children.
var pwrsLayout = (function () {
  'use strict';

  function props(node) { return node.props || {}; }
  function kids(node) { return node.children || []; }

  // Whether a node takes room where it stands: every widget but a menu bar,
  // a status bar and a button bar, which stand along the window's edges,
  // and a context menu, which opens over the window.
  function inFlow(node) {
    return ['MenuBar', 'StatusBar', 'ButtonBar', 'ContextMenu'].indexOf(node.kind) < 0;
  }

  // Whether a node fills down: a list, a table and a tree do; a log, a
  // scroll area, a chart, a GPU surface and a canvas without a height; a
  // media player showing a picture, without a height; a Split standing its
  // children one above the other or with one that fills; a Spacer with
  // fill; a Column, Group, Tabs or Tab with a child that fills; and a Row
  // or Grid with one that is not a Spacer.
  function fillsDown(node) {
    var p = props(node);
    var sized = p.height !== undefined && p.height !== null;
    switch (node.kind) {
      case 'ListBox': case 'Table': case 'TreeView': return true;
      case 'LogView': case 'Scroll': case 'Chart': case 'GpuSurface': case 'Canvas': return !sized;
      case 'MediaPlayer': return !sized && p.audio_only !== true;
      case 'Split': return p.orientation === 'vertical' || kids(node).some(fillsDown);
      case 'Spacer': return p.fill === true;
      case 'Column': case 'Group': case 'Tabs': case 'Tab': return kids(node).some(fillsDown);
      case 'Row': case 'Grid': return kids(node).some(function (c) { return c.kind !== 'Spacer' && fillsDown(c); });
      default: return false;
    }
  }

  // A node's align: start where it has none.
  function align(node) {
    var a = props(node).align;
    if (a === undefined || a === null) return 'start';
    if (['start', 'center', 'end', 'stretch'].indexOf(a) < 0) throw new Error('align must be start, center, end or stretch, not ' + (typeof a === 'string' ? a : JSON.stringify(a)));
    return a;
  }

  // A button's role: 'default', which Enter presses, 'cancel', which Escape
  // presses, or 'other'.
  function role(node) {
    var flag = function (key) {
      var v = props(node)[key];
      if (v === undefined || v === null) return false;
      if (typeof v !== 'boolean') throw new Error(key + ' must be true or false, not ' + JSON.stringify(v));
      return v;
    };
    var isDefault = flag('default'), isCancel = flag('cancel');
    if (isDefault && isCancel) throw new Error('a button is the default or the cancel button, not both');
    return isDefault ? 'default' : isCancel ? 'cancel' : 'other';
  }

  // The order a bar shows buttons of `roles` in, as their places in
  // `roles`: with `windowsOrder` the default button first, then the others
  // in the order given, then the cancel button, as Windows orders a
  // dialog's buttons; else the others, then the cancel button, then the
  // default, as macOS and GTK order them.
  function order(roles, windowsOrder) {
    var of = function (r) { return roles.map(function (x, i) { return x === r ? i : -1; }).filter(function (i) { return i >= 0; }); };
    return windowsOrder ? of('default').concat(of('other'), of('cancel')) : of('other').concat(of('cancel'), of('default'));
  }

  // The key a press stands for in a list, table or tree (proto rows
  // RowKey::named): `key` as a shortcut names it, with Ctrl, Shift and Alt
  // pressed as given; null for a press the widgets leave to the window.
  function rowKeyNamed(key, ctrl, shift, alt) {
    if (alt) return null;
    switch (key) {
      case 'ArrowUp': return 'Up';
      case 'ArrowDown': return 'Down';
      case 'Home': return 'Home';
      case 'End': return 'End';
      case 'PageUp': return 'PageUp';
      case 'PageDown': return 'PageDown';
      case 'Space': return ctrl ? null : 'Space';
      case 'Enter': return ctrl ? null : 'Enter';
      case 'A': return ctrl && !shift ? 'All' : null;
      default: return null;
    }
  }

  // What a key does in a list, table or tree (proto rows row_key). `state`
  // gives count, selection ('none', 'single', 'multiple' or 'checks'),
  // current and anchor (rows as shown, or null), currentSelected, whether
  // the current row is selected, and page. Null when the key does nothing,
  // Enter on a row not selected among them; else the row the keyboard
  // moves to, the range's anchor, the selection made ({ kind: 'same',
  // 'only', 'range', 'toggle' or 'all', at, to }) and whether Enter
  // activates the row.
  function rowKey(key, shift, ctrl, state) {
    if (state.count < 1) return null;
    var last = state.count - 1, sel = state.selection, anchor = state.anchor, step = Math.max(state.page, 1);
    var current = state.current === null ? null : Math.min(state.current, last);
    var made = function (at, from, select, activate) { return { current: at, anchor: from, select: select, activate: activate }; };
    var to;
    switch (key) {
      case 'Up': to = current === null ? 0 : Math.max(current - 1, 0); break;
      case 'Down': to = current === null ? 0 : Math.min(current + 1, last); break;
      case 'Home': to = 0; break;
      case 'End': to = last; break;
      case 'PageUp': to = current === null ? 0 : Math.max(current - step, 0); break;
      case 'PageDown': to = current === null ? 0 : Math.min(current + step, last); break;
      case 'Space': {
        if (current === null || sel === 'none') return null;
        if (sel === 'single') return made(current, current, { kind: 'only', at: current }, false);
        if (sel === 'multiple' && shift) {
          var from = anchor === null ? current : anchor;
          return made(current, from, { kind: 'range', at: from, to: current }, false);
        }
        return made(current, current, { kind: 'toggle', at: current }, false);
      }
      case 'Enter':
        if (current === null || !state.currentSelected || (sel !== 'single' && sel !== 'multiple')) return null;
        return made(current, anchor, { kind: 'same' }, true);
      case 'All':
        if (sel !== 'multiple' && sel !== 'checks') return null;
        return made(current === null ? 0 : current, anchor, { kind: 'all' }, false);
      default: return null;
    }
    if (sel === 'none' || sel === 'checks' || (sel === 'multiple' && ctrl)) return made(to, anchor, { kind: 'same' }, false);
    if (sel === 'multiple' && shift) {
      var start = anchor !== null ? anchor : current !== null ? current : to;
      return made(to, start, { kind: 'range', at: start, to: to }, false);
    }
    return made(to, to, { kind: 'only', at: to }, false);
  }

  // Applies a selection rowKey made to `selected`, a Set of the keys rows
  // are named by, `shown` giving each row's key as shown (proto rows
  // RowSelect::apply); says whether it changed. A row past the end selects
  // nothing.
  function applyRows(select, selected, shown) {
    var before = new Set(selected), i;
    switch (select.kind) {
      case 'only':
        selected.clear();
        if (select.at < shown.length) selected.add(shown[select.at]);
        break;
      case 'range': {
        var lo = Math.min(select.at, select.to), hi = Math.max(select.at, select.to);
        selected.clear();
        for (i = lo; i <= hi && i < shown.length; i++) selected.add(shown[i]);
        break;
      }
      case 'toggle':
        if (select.at < shown.length && !selected.delete(shown[select.at])) selected.add(shown[select.at]);
        break;
      case 'all':
        selected.clear();
        shown.forEach(function (k) { selected.add(k); });
        break;
      default: break;
    }
    if (before.size !== selected.size) return true;
    var changed = false;
    before.forEach(function (k) { if (!selected.has(k)) changed = true; });
    return changed;
  }

  // How many rows a page key moves by in a widget showing `shown` points of
  // rows `row` points apart (proto rows page_rows): those that fit whole,
  // less one, and one at least.
  function pageRows(shown, row) {
    if (!(row > 0) || !isFinite(shown)) return 1;
    return Math.max(Math.floor(shown / row) - 1, 1);
  }

  // The text Ctrl+C copies from a table's rows, as Out-GridView copies its
  // own (proto rows copied): `rows`, each an array of its cells' text, every
  // cell followed by a tab and every row by `newline`, the last of each too.
  function copied(rows, newline) {
    return rows.map(function (row) { return row.map(function (cell) { return cell + '\t'; }).join('') + newline; }).join('');
  }

  // An X as a chart shows it (proto chart x_text): on a chart of dates, the
  // date to the second, with its fraction when it has one, counted from the
  // Unix epoch as the protocol counts it; otherwise the number.
  function xText(x, dates) {
    if (!dates) return String(x);
    var t = new Date(Math.round(x * 1000)).toISOString();
    var fraction = t.slice(20, 23).replace(/0+$/, '');
    return t.slice(0, 19).replace('T', ' ') + (fraction ? '.' + fraction : '');
  }

  // A date axis's label for a grid line at `x`, `step` seconds from the
  // next (proto chart date_tick), naming only what changes along the axis:
  // the day for lines a day or more apart and for a line at midnight, the
  // time to the minute for lines a minute or more apart, and the time to
  // the second, with its fraction when it has one, for closer ones.
  function dateTick(x, step) {
    var full = xText(x, true);
    var midnight = Math.abs(x - Math.round(x / 86400) * 86400) < step / 2;
    if (step >= 86400 || midnight) return full.slice(0, 10);
    if (step >= 60) return full.slice(11, 16);
    return full.slice(11);
  }

  // The widest label dateTick writes for lines `step` seconds apart (proto
  // chart date_sample).
  function dateSample(step) {
    return step >= 86400 ? '0000-00-00' : step >= 60 ? '00:00' : step >= 1 ? '00:00:00' : '00:00:00.000';
  }

  // Whether a password field offers its peek button (proto peek::Peek):
  // once it has the focus and is typed into, and once the focus leaves, not
  // again until it is cleared; text a script sets withholds it until then.
  function peek(text) {
    var armed = text === '', offered = false;
    return {
      set: function (t) { armed = t === ''; offered = false; },
      update: function (focused, t) {
        if (t === '') { armed = true; offered = false; } else if (focused && armed) offered = true;
        if (!focused && offered) { offered = false; armed = false; }
        return offered;
      },
      offered: function () { return offered; }
    };
  }

  return { inFlow: inFlow, fillsDown: fillsDown, align: align, role: role, order: order, rowKeyNamed: rowKeyNamed, rowKey: rowKey, applyRows: applyRows, pageRows: pageRows, copied: copied, xText: xText, dateTick: dateTick, dateSample: dateSample, peek: peek };
})();
