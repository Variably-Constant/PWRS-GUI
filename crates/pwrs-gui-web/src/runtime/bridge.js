// window.pwrs, run before any script of a page: send posts a value to the
// session and returns a Promise of the session's reply, on registers a
// handler for values the session sends, close closes the window, giving
// the session a value, save stores a value for the page to get back from
// restore once it is loaded again after its engine ended, and restore
// returns a Promise of that value, null when nothing was saved.
// __loadErrors has the message of each error raised before the page
// finished loading, so the widget runtime can say what stopped a script it
// draws with from running.
(function () {
  var handlers = [];
  var waiting = {};
  var restoring = [];
  var loadErrors = [];
  var last = 0;
  function post(message) { window.ipc.postMessage(JSON.stringify(message)); }
  function json(value) { return value === undefined ? null : value; }
  function raised(e) { loadErrors.push(String(e.message)); }
  window.addEventListener('error', raised);
  window.addEventListener('load', function () { window.removeEventListener('error', raised); });
  window.pwrs = {
    send: function (value) {
      last = last % 4294967295 + 1;
      var call = last;
      return new Promise(function (resolve) {
        waiting[call] = resolve;
        post({ t: 'message', call: call, value: json(value) });
      });
    },
    on: function (handler) { handlers.push(handler); },
    close: function (value) { post({ t: 'pageclose', value: json(value) }); },
    save: function (value) { post({ t: 'save', value: json(value) }); },
    restore: function () {
      return new Promise(function (resolve) {
        restoring.push(resolve);
        post({ t: 'restore' });
      });
    },
    __post: post,
    __loadErrors: loadErrors,
    __deliver: function (value) { handlers.forEach(function (h) { h(value); }); },
    __reply: function (call, value) {
      var resolve = waiting[call];
      if (resolve) { delete waiting[call]; resolve(value); }
    },
    __restored: function (value) {
      var answered = restoring;
      restoring = [];
      answered.forEach(function (resolve) { resolve(value); });
    }
  };
})();
