// window.pwrs, run before any script of a page: send posts a value to the
// session and returns a Promise of the session's reply, call asks the
// session to run what the window allows under a name with the arguments
// given and returns a Promise of its output, rejected with an Error when it
// fails or is refused, on registers a handler for values the session sends,
// channel opens a channel to the page's backend over two SubEtha rings,
// close closes the window, giving the session a value, save stores a value
// for the page to get back from restore once it is loaded again after its
// engine ended, and restore returns a Promise of that value, null when
// nothing was saved. window acts on the page's own window, whose title bar
// a page without the system's frame draws itself, and on the other windows
// of its app, which it opens; ondrop registers a handler for the full paths
// of files dropped on the page. Sends, calls, opens and lists take their
// numbers from one count.
// A page's elements marked data-pwrs-drag move its window when pressed,
// and a frameless page's edges resize it, as window.__pwrsPage says, which
// the helper sets before the page's own scripts run.
// __input waits for a step of a test's input, for the widget runtime and a
// page of the caller's own alike, and __buttonsOf gives the buttons held at
// a pointer event, those a test's input holds among them.
// __loadErrors has the message of each error raised before the page
// finished loading, so the widget runtime can say what stopped a script it
// draws with from running.
(function () {
  var handlers = [];
  var waiting = {};
  var calling = {};
  var restoring = [];
  var loadErrors = [];
  var windowHandlers = {};
  var closeHandlers = [];
  var last = 0;
  function post(message) { window.ipc.postMessage(JSON.stringify(message)); }
  function json(value) { return value === undefined ? null : value; }
  function raised(e) { loadErrors.push(String(e.message)); }
  function next() { last = last % 4294967295 + 1; return last; }
  window.addEventListener('error', raised);
  window.addEventListener('load', function () { window.removeEventListener('error', raised); });

  // The buttons a test's input holds, as a pointer event's buttons bits.
  // They matter on an engine that takes the buttons of every event a program
  // makes from the hardware, which then reports none held: WebKit on macOS.
  // There the helper gives each step the buttons held before it and those it
  // leaves held (__input). The second take over at the step's own press or
  // release, the first event of its button (stepHeld). On other engines no
  // step gives them, and only the event's own are read.
  var testHeld = 0, stepHeld = null;
  // The buttons held when `e` came: its own, with those a test's input holds.
  function buttonsOf(e) { return e.buttons | testHeld; }
  // This listens on the window before any other listener of the page's
  // pointer events, the widget runtime's and the page's own, so each one
  // after it reads the buttons held as a step's own event leaves them.
  ['pointerdown', 'pointermove', 'pointerup'].forEach(function (type) {
    window.addEventListener(type, function (e) {
      if (!e.isTrusted || stepHeld === null || e.button !== stepHeld.button) return;
      testHeld = stepHeld.after;
      stepHeld = null;
    }, true);
  });

  // The step of a test's input the page waits to see (__input), { ask,
  // step }, or null; and where the latest of the person's pointer events
  // put the pointer. A step is seen at the event the engine raises for it.
  // A press or a release is seen as its button's bit comes into or goes out
  // of the buttons held, and a move at a move to its point with no button
  // changing. A key is seen at its release, or for a key that opens a
  // context menu in the engine, at the contextmenu event that takes its
  // release's place.
  var awaited = null, lastPointer = null;
  var BUTTON_BITS = { 0: 1, 1: 4, 2: 2 };
  function awaitedBy(e, s) {
    if (s.kind === 'key') return e.type === 'keyup' || e.type === 'contextmenu';
    if (e.type !== 'pointerdown' && e.type !== 'pointermove' && e.type !== 'pointerup') return false;
    if (s.kind === 'move') return e.type === 'pointermove' && e.button === -1 && Math.abs(e.clientX - s.x) <= 1 && Math.abs(e.clientY - s.y) <= 1;
    if (e.button !== s.button) return false;
    return ((buttonsOf(e) & BUTTON_BITS[s.button]) !== 0) === (s.kind === 'press');
  }
  // Reports the step of a test's input numbered `ask` seen once two frames
  // have run after it, so the page has acted on it and drawn what it did.
  function inputSeen(ask) {
    requestAnimationFrame(function () {
      requestAnimationFrame(function () { post({ t: 'input', request: ask, seen: true }); });
    });
  }
  ['pointerdown', 'pointermove', 'pointerup', 'keyup', 'contextmenu'].forEach(function (type) {
    window.addEventListener(type, function (e) {
      if (!e.isTrusted) return;
      if (e.type.indexOf('pointer') === 0) lastPointer = [e.clientX, e.clientY];
      if (!awaited || !awaitedBy(e, awaited.step)) return;
      var ask = awaited.ask;
      awaited = null;
      inputSeen(ask);
    }, true);
  });

  // An element marked data-pwrs-drag moves its window when pressed with
  // the primary button, but for the links, buttons, fields and editable
  // text inside it and what is marked data-pwrs-no-drag; a double-click
  // there maximizes the window or restores it. On WebView2 such an element
  // is also the window's caption to the system (app-region), which moves,
  // snaps and maximizes the window itself, so the page sees no press there
  // but a test's.
  var KEEPS_PRESSES = 'a, button, input, select, textarea, [contenteditable], [data-pwrs-no-drag]';
  function dragRegion(target) {
    if (!(target instanceof Element)) return null;
    var region = target.closest('[data-pwrs-drag]');
    if (!region) return null;
    var inner = target.closest(KEEPS_PRESSES);
    return inner && region.contains(inner) ? null : region;
  }
  window.addEventListener('mousedown', function (e) {
    if (e.button !== 0 || e.detail > 1 || !dragRegion(e.target)) return;
    post({ t: 'window', op: 'drag' });
  }, true);
  window.addEventListener('dblclick', function (e) {
    if (e.button === 0 && dragRegion(e.target)) post({ t: 'window', op: 'toggleMaximize' });
  }, true);

  // A frameless page's edges, as window.__pwrsPage names them: a strip
  // along each edge and a square at each corner, above all the page draws,
  // in a closed shadow root so the page's own styles reach none of them,
  // each starting the system's resize from its edge when pressed.
  var CURSORS = { n: 'ns-resize', s: 'ns-resize', e: 'ew-resize', w: 'ew-resize', ne: 'nesw-resize', sw: 'nesw-resize', nw: 'nwse-resize', se: 'nwse-resize' };
  function placeEdges(edges) {
    var host = document.createElement('pwrs-frame');
    var root = host.attachShadow({ mode: 'closed' });
    edges.forEach(function (edge) {
      var handle = document.createElement('div');
      var s = handle.style;
      var down = edge[0] === 'n' || edge[0] === 's' ? edge[0] : '';
      var across = edge[edge.length - 1] === 'e' || edge[edge.length - 1] === 'w' ? edge[edge.length - 1] : '';
      var corner = down !== '' && across !== '';
      s.position = 'fixed';
      s.zIndex = '2147483647';
      s.cursor = CURSORS[edge];
      if (down) {
        s[down === 'n' ? 'top' : 'bottom'] = '0';
        s.height = (corner ? 8 : 5) + 'px';
      } else {
        s.top = '8px';
        s.bottom = '8px';
      }
      if (across) {
        s[across === 'w' ? 'left' : 'right'] = '0';
        s.width = (corner ? 8 : 5) + 'px';
      } else {
        s.left = '8px';
        s.right = '8px';
      }
      handle.addEventListener('pointerdown', function (e) {
        if (e.button !== 0) return;
        e.preventDefault();
        post({ t: 'window', op: 'resize', edge: edge });
      });
      root.appendChild(handle);
    });
    document.documentElement.appendChild(host);
  }
  document.addEventListener('DOMContentLoaded', function () {
    var frame = window.__pwrsPage;
    if (window.chrome && window.chrome.webview) {
      var sheet = document.createElement('style');
      sheet.textContent = '[data-pwrs-drag] { app-region: drag; -webkit-app-region: drag; } [data-pwrs-drag] :is(' + KEEPS_PRESSES + ') { app-region: no-drag; -webkit-app-region: no-drag; }';
      (document.head || document.documentElement).appendChild(sheet);
    }
    if (frame && frame.frameless && frame.edges.length) placeEdges(frame.edges);
  });

  // Files dropped on the page, whose own handlers of its elements and
  // document see each drop event first, as in a browser. The bridge then
  // tells the helper of a drop of files: whether the page took it with
  // preventDefault, and where. On WebView2 the message is an object whose
  // additional objects are the files, from which the helper reads their
  // full paths; elsewhere the helper has the paths from the system's drop.
  // Where the page takes no drop, it goes ahead when the window's command
  // takes drops, as window.__pwrsPage says, or the page has an ondrop
  // handler. A test's drop (testing) goes as a plain message, since its
  // files were made in the page and have no paths to read.
  var dropHandlers = [];
  var testing = false;
  function filesIn(e) {
    var types = e.dataTransfer && e.dataTransfer.types;
    return !!types && Array.prototype.indexOf.call(types, 'Files') >= 0;
  }
  function dropsWanted() { return dropHandlers.length > 0 || !!(window.__pwrsPage && window.__pwrsPage.drop); }
  window.addEventListener('dragover', function (e) {
    if (!filesIn(e) || e.defaultPrevented || !dropsWanted()) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
  });
  window.addEventListener('drop', function (e) {
    if (!filesIn(e)) return;
    var taken = e.defaultPrevented;
    var files = Array.prototype.slice.call(e.dataTransfer.files);
    if (!taken) e.preventDefault();
    var message = { t: 'drop', taken: taken, x: e.clientX, y: e.clientY };
    var view = window.chrome && window.chrome.webview;
    if (!testing && view && typeof view.postMessageWithAdditionalObjects === 'function') view.postMessageWithAdditionalObjects(message, files);
    else post(message);
  });
  // A test's drop of files on the page at (x, y), landing as a person's
  // would: files of the names given, made in the page, whose full paths
  // the helper has from the test.
  function testDrop(names, x, y) {
    var data = new DataTransfer();
    names.forEach(function (name) { data.items.add(new File([], name)); });
    var target = document.elementFromPoint(x, y) || document.documentElement;
    testing = true;
    try {
      ['dragenter', 'dragover', 'drop'].forEach(function (type) {
        target.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, clientX: x, clientY: y, dataTransfer: data }));
      });
    } finally {
      testing = false;
    }
  }

  // A window of the page's app, as open and list give one and of makes one:
  // its number, as text, since a JavaScript number cannot hold every
  // window's number exactly; its title as open or list gave it; and what
  // the page may ask of it, as of its own window. A close is as its
  // person's, which its own page's onclose handlers decide.
  function appWindow(id, title) {
    var target = String(id);
    function ask(message) {
      message.t = 'window';
      message.target = target;
      post(message);
    }
    return {
      id: target,
      title: title === undefined ? null : title,
      minimize: function () { ask({ op: 'minimize' }); },
      maximize: function () { ask({ op: 'maximize' }); },
      restore: function () { ask({ op: 'restore' }); },
      toggleMaximize: function () { ask({ op: 'toggleMaximize' }); },
      close: function () { ask({ op: 'close' }); },
      move: function (x, y) { ask({ op: 'moveTo', x: x, y: y }); },
      resize: function (width, height) { ask({ op: 'resizeTo', width: width, height: height }); },
      focus: function () { ask({ op: 'focus' }); },
      fullscreen: function (on) { ask({ op: 'fullscreen', on: on !== false }); },
      setTitle: function (text) { ask({ op: 'setTitle', title: String(text) }); }
    };
  }
  // The page's channels to its backend, by name, each one object while it
  // is open. A channel is reached at the page's own origin, which
  // window.__pwrsPage names, through POST requests to /.pwrs/<name>/<what>:
  // open, send of one ring frame, recv of the backend's frames, each behind
  // its length in four bytes, least significant first, and close.
  var channels = {};
  function channelAt(name, what) {
    var origin = window.__pwrsPage ? window.__pwrsPage.origin : '';
    return origin + '/.pwrs/' + encodeURIComponent(name) + '/' + what;
  }
  // What a channel sends, as bytes: a string as UTF-8, or the bytes of an
  // ArrayBuffer, a typed array or a Blob.
  function bytesOf(data) {
    if (typeof data === 'string') return Promise.resolve(new TextEncoder().encode(data));
    if (data instanceof ArrayBuffer) return Promise.resolve(new Uint8Array(data));
    if (ArrayBuffer.isView(data)) return Promise.resolve(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
    if (typeof Blob !== 'undefined' && data instanceof Blob) return data.arrayBuffer().then(function (b) { return new Uint8Array(b); });
    return Promise.reject(new TypeError('a channel sends a string, an ArrayBuffer, a typed array or a Blob'));
  }
  function joined(pieces) {
    var length = pieces.reduce(function (n, p) { return n + p.byteLength; }, 0);
    var out = new Uint8Array(length), at = 0;
    pieces.forEach(function (p) { out.set(p, at); at += p.byteLength; });
    return out;
  }
  // A channel, as a WebSocket is: readyState, binaryType, bufferedAmount,
  // onopen, onmessage, onclose, onerror, send and close. A message channel
  // sends each message as parts of one ring frame each, whose first byte
  // says whole (0), more (1) or last (2), and gives the page each message
  // whole, as text on a text channel; a stream channel sends and gives
  // plain frames. send returns a Promise settled once its frames are in
  // the ring, and waits while the frames and bytes in flight are at the
  // channel's caps.
  function Channel(name) {
    var ch = this;
    var settings = null, queue = [], flying = { frames: 0, bytes: 0 }, handled = { frames: 0, bytes: 0 }, parts = [], ended = false;
    ch.name = name;
    ch.readyState = 0;
    ch.binaryType = 'arraybuffer';
    ch.bufferedAmount = 0;
    ch.onopen = null;
    ch.onmessage = null;
    ch.onclose = null;
    ch.onerror = null;
    function ask(what, body) { return fetch(channelAt(name, what), { method: 'POST', body: body }); }
    function end(reason) {
      if (ended) return;
      ended = true;
      ch.readyState = 3;
      if (channels[name] === ch) delete channels[name];
      queue.forEach(function (q) { q.reject(new Error(reason)); });
      queue = [];
      if (typeof ch.onclose === 'function') ch.onclose({ type: 'close', reason: reason });
    }
    function failed(reason) {
      if (typeof ch.onerror === 'function') ch.onerror({ type: 'error', message: reason });
      end(reason);
    }
    function deliver(bytes) {
      var data;
      if (settings.text) data = new TextDecoder().decode(bytes);
      else if (ch.binaryType === 'blob') data = new Blob([bytes]);
      else data = bytes.slice().buffer;
      if (typeof ch.onmessage === 'function') ch.onmessage({ type: 'message', data: data });
    }
    function received(frame) {
      handled.frames += 1;
      handled.bytes += frame.byteLength;
      if (settings.stream) { deliver(frame); return; }
      if (frame.byteLength === 0) return;
      var kind = frame[0], body = frame.subarray(1);
      if (kind === 0) { parts = []; deliver(body); }
      else if (kind === 1) parts.push(body.slice());
      else if (kind === 2) { parts.push(body); var whole = joined(parts); parts = []; deliver(whole); }
    }
    function poll() {
      if (ended) return;
      var gave = JSON.stringify(handled);
      handled = { frames: 0, bytes: 0 };
      ask('recv', gave).then(function (r) {
        if (!r.ok) return r.text().then(function (why) { end(why || 'the channel ended'); });
        return r.arrayBuffer().then(function (b) {
          var view = new DataView(b), all = new Uint8Array(b), at = 0;
          while (at + 4 <= b.byteLength) {
            var n = view.getUint32(at, true);
            at += 4;
            received(all.subarray(at, at + n));
            at += n;
          }
          poll();
        });
      }, function (e) { end(String(e && e.message || e)); });
    }
    function framesOf(bytes) {
      var size = settings.frameBytes, out = [], at;
      if (settings.stream) {
        for (at = 0; at < bytes.byteLength; at += size) out.push(bytes.slice(at, at + size));
        return out;
      }
      var room = size - 1;
      if (bytes.byteLength <= room) return [joined([new Uint8Array([0]), bytes])];
      for (at = 0; at < bytes.byteLength; at += room) {
        var last = at + room >= bytes.byteLength;
        out.push(joined([new Uint8Array([last ? 2 : 1]), bytes.subarray(at, at + room)]));
      }
      return out;
    }
    function pump() {
      while (queue.length && !ended) {
        var q = queue[0];
        if (flying.frames > 0 && (flying.frames + 1 > settings.maxFrames || flying.bytes + q.frame.byteLength > settings.maxBytes)) return;
        queue.shift();
        flying.frames += 1;
        flying.bytes += q.frame.byteLength;
        sendFrame(q);
      }
    }
    function sendFrame(q) {
      function landed() {
        flying.frames -= 1;
        flying.bytes -= q.frame.byteLength;
        ch.bufferedAmount -= q.frame.byteLength;
      }
      ask('send', q.frame).then(function (r) {
        landed();
        if (r.ok) { q.resolve(); pump(); return; }
        return r.text().then(function (why) { q.reject(new Error(why)); pump(); });
      }, function (e) { landed(); q.reject(e); pump(); });
    }
    ch.send = function (data) {
      if (ch.readyState !== 1) return Promise.reject(new Error('the channel ' + name + ' is not open'));
      return bytesOf(data).then(function (bytes) {
        var sent = framesOf(bytes).map(function (frame) {
          return new Promise(function (resolve, reject) {
            queue.push({ frame: frame, resolve: resolve, reject: reject });
            ch.bufferedAmount += frame.byteLength;
          });
        });
        pump();
        return Promise.all(sent).then(function () { return undefined; });
      });
    };
    ch.close = function () {
      if (ended) return;
      ch.readyState = 2;
      ask('close', '').then(function () { end('the page closed the channel'); }, function () { end('the page closed the channel'); });
    };
    ask('open', '').then(function (r) {
      if (!r.ok) return r.text().then(failed);
      return r.json().then(function (s) {
        settings = s;
        ch.readyState = 1;
        if (typeof ch.onopen === 'function') ch.onopen({ type: 'open' });
        poll();
      });
    }, function (e) { failed(String(e && e.message || e)); });
  }

  // A Promise settled through __answer, as a call's is: the message `build`
  // makes of its number is posted, and what the answer gives is resolved
  // as `settle` makes it.
  function asking(build, settle) {
    var call = next();
    return new Promise(function (resolve, reject) {
      calling[call] = { resolve: function (value) { resolve(settle(value)); }, reject: reject };
      post(build(call));
    });
  }

  window.pwrs = {
    send: function (value) {
      var call = next();
      return new Promise(function (resolve) {
        waiting[call] = resolve;
        post({ t: 'message', call: call, value: json(value) });
      });
    },
    call: function (name, args) {
      if (typeof name !== 'string' || name === '') return Promise.reject(new TypeError('pwrs.call takes the name of what it runs, as a string'));
      var call = next();
      return new Promise(function (resolve, reject) {
        calling[call] = { resolve: resolve, reject: reject };
        post({ t: 'call', call: call, name: name, args: json(args) });
      });
    },
    on: function (handler) { handlers.push(handler); },
    // The page's channel to its backend under `name`, which the window's
    // command offers: opened the first time, and the same object while it
    // stays open.
    channel: function (name) {
      if (typeof name !== 'string' || name === '') throw new TypeError('pwrs.channel takes the name of a channel the window offers, as a string');
      if (!channels[name]) channels[name] = new Channel(name);
      return channels[name];
    },
    // Registers a handler for each drop of files on the page, given
    // { paths, x, y, taken }: the files' full paths, where they landed, and
    // whether the page's own handlers took the drop with preventDefault.
    ondrop: function (handler) { dropHandlers.push(handler); },
    close: function (value) { post({ t: 'pageclose', value: json(value) }); },
    // The page's own window: minimized, maximized, restored, maximized or
    // restored by turns, closed as its person closes it, and moved or
    // resized by the system from an edge (n, ne, e, se, s, sw, w, nw) while
    // the button that pressed the page stays held. move and resize put its
    // top left at a point of the screen and make its inside a size, in
    // points, leaving 48 points of it on a monitor; focus gives it the
    // keyboard as far as the system allows; fullscreen fills its monitor
    // with it, or with false undoes that; setTitle names it. on registers a
    // handler for its move, resize, focus or blur, and onclose one that a
    // close as its person's is given first, to keep the window open with
    // preventDefault. buttons is the box of the system's own buttons over a
    // frameless page, in points from its top left, which the page leaves
    // room for: macOS's three, unless hidden; null where there are none.
    // id is the window's number, as text. open opens another window of the
    // app, a Promise of it (appWindow) once it is open; list is a Promise of
    // the app's open windows; of(id) is the app's window of that number.
    window: {
      get id() { return window.__pwrsPage ? window.__pwrsPage.id : null; },
      open: function (options) {
        return asking(function (call) { return { t: 'open', call: call, options: json(options) }; }, function (w) { return appWindow(w.id, w.title); });
      },
      list: function () {
        return asking(function (call) { return { t: 'window', op: 'list', call: call }; }, function (ws) { return ws.map(function (w) { return appWindow(w.id, w.title); }); });
      },
      of: function (id) { return appWindow(id); },
      minimize: function () { post({ t: 'window', op: 'minimize' }); },
      maximize: function () { post({ t: 'window', op: 'maximize' }); },
      restore: function () { post({ t: 'window', op: 'restore' }); },
      toggleMaximize: function () { post({ t: 'window', op: 'toggleMaximize' }); },
      close: function () { post({ t: 'window', op: 'close' }); },
      startDrag: function () { post({ t: 'window', op: 'drag' }); },
      startResize: function (edge) { post({ t: 'window', op: 'resize', edge: edge }); },
      move: function (x, y) { post({ t: 'window', op: 'moveTo', x: x, y: y }); },
      resize: function (width, height) { post({ t: 'window', op: 'resizeTo', width: width, height: height }); },
      focus: function () { post({ t: 'window', op: 'focus' }); },
      fullscreen: function (on) { post({ t: 'window', op: 'fullscreen', on: on !== false }); },
      setTitle: function (title) { post({ t: 'window', op: 'setTitle', title: String(title) }); },
      on: function (kind, handler) { (windowHandlers[kind] = windowHandlers[kind] || []).push(handler); },
      onclose: function (handler) {
        if (!closeHandlers.length) post({ t: 'window', op: 'holdClose' });
        closeHandlers.push(handler);
      },
      get buttons() {
        var b = window.__pwrsPage && window.__pwrsPage.buttons;
        return b ? { x: b.x, y: b.y, width: b.width, height: b.height } : null;
      }
    },
    save: function (value) { post({ t: 'save', value: json(value) }); },
    restore: function () {
      return new Promise(function (resolve) {
        restoring.push(resolve);
        post({ t: 'restore' });
      });
    },
    __post: post,
    __buttonsOf: buttonsOf,
    __loadErrors: loadErrors,
    __deliver: function (value) { handlers.forEach(function (h) { h(value); }); },
    __reply: function (call, value) {
      var resolve = waiting[call];
      if (resolve) { delete waiting[call]; resolve(value); }
    },
    // The session's answer to call `call`: its output when `resolved`, else
    // an object whose message says why, given to the Error the Promise
    // rejects with along with the object's other keys.
    __answer: function (call, resolved, value) {
      var owed = calling[call];
      if (!owed) return;
      delete calling[call];
      if (resolved) { owed.resolve(value); return; }
      var given = value !== null && typeof value === 'object' ? value : { message: String(value) };
      var error = new Error(typeof given.message === 'string' ? given.message : JSON.stringify(given));
      Object.keys(given).forEach(function (k) { if (k !== 'message') error[k] = given[k]; });
      owed.reject(error);
    },
    __restored: function (value) {
      var answered = restoring;
      restoring = [];
      answered.forEach(function (resolve) { resolve(value); });
    },
    __dropped: function (drop) { dropHandlers.forEach(function (h) { h(drop); }); },
    __testDrop: testDrop,
    __windowEvent: function (kind, data) { (windowHandlers[kind] || []).forEach(function (h) { h(data); }); },
    // A close of the window as its person's, which the onclose handlers
    // decide: the window closes unless one of them calls preventDefault,
    // and the helper is told either way, since a page that gives no answer
    // in 10 seconds has its window closed.
    __closing: function () {
      var kept = false;
      var e = { preventDefault: function () { kept = true; } };
      closeHandlers.forEach(function (h) { h(e); });
      post(kept ? { t: 'window', op: 'closeKept' } : { t: 'window', op: 'close', confirmed: true });
    },
    // A step of a test's input, numbered `ask`, which the helper gives the
    // engine once the page waits for it. The page says it waits, then that
    // it has seen it (awaited). A move to where the pointer is already is
    // seen at once, since no engine raises one, and a wait once its seconds
    // have passed. A step of null stops the waiting for step `ask`. A step's
    // `held`, given where the engine takes the buttons from the hardware, is
    // the buttons held before it and those it leaves held (testHeld).
    __input: function (ask, step) {
      if (step === null) {
        if (awaited && awaited.ask === ask) awaited = null;
        return;
      }
      if (step.held) {
        testHeld = step.held.before;
        stepHeld = step.held.after === step.held.before ? null : { button: step.button, after: step.held.after };
      }
      if (step.kind === 'wait') {
        setTimeout(function () { inputSeen(ask); }, step.seconds * 1000);
        return;
      }
      var here = step.kind === 'move' && lastPointer !== null && Math.abs(lastPointer[0] - step.x) < 0.5 && Math.abs(lastPointer[1] - step.y) < 0.5;
      awaited = here ? null : { ask: ask, step: step };
      post({ t: 'input', request: ask, armed: true });
      if (here) inputSeen(ask);
    }
  };
})();
