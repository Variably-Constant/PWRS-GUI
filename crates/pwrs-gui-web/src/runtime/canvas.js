// A canvas's arithmetic, as the protocol crate has it for egui: the
// easing curves an item's Animate and a view's moves follow (proto
// canvas.rs, Easing::at), and a view's placing, wheel, drags, steps, box,
// glide and eased moves (proto view.rs). The numbers a view moves by come
// from the helper, which takes them from the protocol crate, as do the
// gestures and keys a view's controls bind, written as
// ViewGesture::written writes them. The pointer's clicks and drags are
// counted and settled as egui 0.37 settles them, and the widget a press
// goes to is found as egui's hit test finds it. Lengths on the canvas are
// in points.
var pwrsCanvas = (function () {
  'use strict';

  var PI = Math.PI;

  // The cubic Bézier from 0,0 to 1,1 through x1,y1 and x2,y2, as CSS's
  // cubic-bezier() takes it, at x: its y where its x is x, found by
  // Newton's method from x, then by halving where that does not settle.
  function bezier(x1, y1, x2, y2, x) {
    function along(a, b, s) { return 3 * a * s * (1 - s) * (1 - s) + 3 * b * s * s * (1 - s) + s * s * s; }
    function slope(a, b, s) { return 3 * a * (1 - s) * (1 - s) + 6 * (b - a) * s * (1 - s) + 3 * (1 - b) * s * s; }
    var s = x, i;
    for (i = 0; i < 8; i++) {
      var gap = along(x1, x2, s) - x;
      if (Math.abs(gap) < 1e-9) return along(y1, y2, s);
      var d = slope(x1, x2, s);
      if (Math.abs(d) < 1e-9) break;
      s -= gap / d;
    }
    var low = 0, high = 1;
    s = x;
    for (i = 0; i < 64; i++) {
      var at = along(x1, x2, s);
      if (Math.abs(at - x) < 1e-9) break;
      if (at < x) low = s; else high = s;
      s = (low + high) / 2;
    }
    return along(y1, y2, s);
  }

  // Penner's in-out curve of power n at t.
  function inOut(t, n) { return t < 0.5 ? Math.pow(2, n - 1) * Math.pow(t, n) : 1 - Math.pow(-2 * t + 2, n) / 2; }

  // easings.net's out-bounce at t.
  function bounce(t) {
    var n = 7.5625, d = 2.75, u;
    if (t < 1 / d) return n * t * t;
    if (t < 2 / d) { u = t - 1.5 / d; return n * u * u + 0.75; }
    if (t < 2.5 / d) { u = t - 2.25 / d; return n * u * u + 0.9375; }
    u = t - 2.625 / d;
    return n * u * u + 0.984375;
  }

  var BACK = 1.70158, ELASTIC = 2 * PI / 3;

  // Every curve by its name: CSS's five, then Penner's, as easings.net
  // names them.
  var curves = {
    Linear: function (t) { return t; },
    Ease: function (t) { return bezier(0.25, 0.1, 0.25, 1, t); },
    EaseIn: function (t) { return bezier(0.42, 0, 1, 1, t); },
    EaseOut: function (t) { return bezier(0, 0, 0.58, 1, t); },
    EaseInOut: function (t) { return bezier(0.42, 0, 0.58, 1, t); },
    EaseInSine: function (t) { return 1 - Math.cos(t * PI / 2); },
    EaseOutSine: function (t) { return Math.sin(t * PI / 2); },
    EaseInOutSine: function (t) { return -(Math.cos(PI * t) - 1) / 2; },
    EaseInQuad: function (t) { return Math.pow(t, 2); },
    EaseOutQuad: function (t) { return 1 - Math.pow(1 - t, 2); },
    EaseInOutQuad: function (t) { return inOut(t, 2); },
    EaseInCubic: function (t) { return Math.pow(t, 3); },
    EaseOutCubic: function (t) { return 1 - Math.pow(1 - t, 3); },
    EaseInOutCubic: function (t) { return inOut(t, 3); },
    EaseInQuart: function (t) { return Math.pow(t, 4); },
    EaseOutQuart: function (t) { return 1 - Math.pow(1 - t, 4); },
    EaseInOutQuart: function (t) { return inOut(t, 4); },
    EaseInQuint: function (t) { return Math.pow(t, 5); },
    EaseOutQuint: function (t) { return 1 - Math.pow(1 - t, 5); },
    EaseInOutQuint: function (t) { return inOut(t, 5); },
    EaseInExpo: function (t) { return t === 0 ? 0 : Math.pow(2, 10 * t - 10); },
    EaseOutExpo: function (t) { return t === 1 ? 1 : 1 - Math.pow(2, -10 * t); },
    EaseInOutExpo: function (t) {
      if (t === 0 || t === 1) return t;
      return t < 0.5 ? Math.pow(2, 20 * t - 10) / 2 : (2 - Math.pow(2, -20 * t + 10)) / 2;
    },
    EaseInCirc: function (t) { return 1 - Math.sqrt(1 - Math.pow(t, 2)); },
    EaseOutCirc: function (t) { return Math.sqrt(1 - Math.pow(t - 1, 2)); },
    EaseInOutCirc: function (t) { return t < 0.5 ? (1 - Math.sqrt(1 - Math.pow(2 * t, 2))) / 2 : (Math.sqrt(1 - Math.pow(-2 * t + 2, 2)) + 1) / 2; },
    EaseInBack: function (t) { return (BACK + 1) * Math.pow(t, 3) - BACK * Math.pow(t, 2); },
    EaseOutBack: function (t) { return 1 + (BACK + 1) * Math.pow(t - 1, 3) + BACK * Math.pow(t - 1, 2); },
    EaseInOutBack: function (t) {
      var c = BACK * 1.525;
      return t < 0.5 ? Math.pow(2 * t, 2) * ((c + 1) * 2 * t - c) / 2 : (Math.pow(2 * t - 2, 2) * ((c + 1) * (t * 2 - 2) + c) + 2) / 2;
    },
    EaseInElastic: function (t) { return t === 0 || t === 1 ? t : -Math.pow(2, 10 * t - 10) * Math.sin((t * 10 - 10.75) * ELASTIC); },
    EaseOutElastic: function (t) { return t === 0 || t === 1 ? t : Math.pow(2, -10 * t) * Math.sin((t * 10 - 0.75) * ELASTIC) + 1; },
    EaseInOutElastic: function (t) {
      if (t === 0 || t === 1) return t;
      var c = 2 * PI / 4.5;
      return t < 0.5 ? -(Math.pow(2, 20 * t - 10) * Math.sin((20 * t - 11.125) * c)) / 2 : Math.pow(2, -20 * t + 10) * Math.sin((20 * t - 11.125) * c) / 2 + 1;
    },
    EaseInBounce: function (t) { return 1 - bounce(1 - t); },
    EaseOutBounce: bounce,
    EaseInOutBounce: function (t) { return t < 0.5 ? (1 - bounce(1 - 2 * t)) / 2 : (1 + bounce(2 * t - 1)) / 2; }
  };

  // How far along the curve named `name` is `t` of the way through, `t`
  // cut to the range 0 to 1.
  function ease(name, t) {
    var curve = curves[name];
    if (!curve) throw new Error("'" + name + "' is no easing");
    return curve(Math.min(1, Math.max(0, t)));
  }

  function clamp(v, low, high) { return Math.min(high, Math.max(low, v)); }

  // A view: { center: [x, y] or null for the canvas's own middle, zoom,
  // minZoom, maxZoom, bindings: { 'Gesture': 'Move' } }, as the helper
  // sends one.
  function copy(v) { return { center: v.center ? v.center.slice() : null, zoom: v.zoom, minZoom: v.minZoom, maxZoom: v.maxZoom, bindings: v.bindings }; }
  function centerIn(v, size) { return v.center ? v.center : [size[0] / 2, size[1] / 2]; }

  // The transform from the items' coordinates to points on a canvas `size`
  // points across and down, as [a, b, c, d, e, f].
  function transform(v, size) {
    var z = v.zoom, c = centerIn(v, size);
    return [z, 0, 0, z, size[0] / 2 - c[0] * z, size[1] / 2 - c[1] * z];
  }

  function toItems(v, at, size) {
    var c = centerIn(v, size);
    return [(at[0] - size[0] / 2) / v.zoom + c[0], (at[1] - size[1] / 2) / v.zoom + c[1]];
  }

  function pan(v, across, down, size) {
    var c = centerIn(v, size);
    v.center = [c[0] - across / v.zoom, c[1] - down / v.zoom];
  }

  function zoomAbout(v, factor, at, size) {
    var before = toItems(v, at, size);
    v.zoom = clamp(v.zoom * factor, v.minZoom, v.maxZoom);
    var after = toItems(v, at, size), c = centerIn(v, size);
    v.center = [c[0] + before[0] - after[0], c[1] + before[1] - after[1]];
  }

  // A wheel turn moving a page's content `by` points across and down, as
  // ViewSpec::wheel makes it, with the view's numbers `k`.
  function wheel(v, made, by, at, size, k) {
    if (made === 'Zoom') zoomAbout(v, Math.pow(k.zoomStep, (by[0] + by[1]) / k.scrollStep), at, size);
    else if (made === 'Scroll') pan(v, by[0], by[1], size);
    else if (made === 'ScrollAcross') pan(v, by[0] + by[1], 0, size);
  }

  // A drag's move of the pointer across and down from `from`, as
  // ViewSpec::dragged makes it.
  function dragged(v, made, across, down, from, size, k) {
    if (made === 'Pan' || made === 'Glide') pan(v, across, down, size);
    else if (made === 'Zoom') zoomAbout(v, Math.pow(k.zoomStep, -down / k.scrollStep), from, size);
  }

  function fit(v, bounds, size) {
    var w = bounds[2], h = bounds[3];
    v.center = [bounds[0] + w / 2, bounds[1] + h / 2];
    var zoom = Math.min(w > 0 ? size[0] / w : Infinity, h > 0 ? size[1] / h : Infinity);
    if (isFinite(zoom)) v.zoom = clamp(zoom, v.minZoom, v.maxZoom);
  }

  // The view a step ends at, as ViewSpec::stepped gives it: null for a
  // move that is no step, an arrow's pan without an arrow, or a fit with
  // nothing to fit. `arrow` is [across, down] for an arrow key.
  function stepped(v, made, at, arrow, bounds, size, k) {
    var to = copy(v);
    if (made === 'ZoomIn') zoomAbout(to, k.zoomInStep, at, size);
    else if (made === 'ZoomOut') zoomAbout(to, 1 / k.zoomInStep, at, size);
    else if (made === 'ActualSize') zoomAbout(to, 1 / v.zoom, at, size);
    else if (made === 'Fit') { if (!bounds) return null; fit(to, bounds, size); }
    else if (made === 'Pan') { if (!arrow) return null; pan(to, -arrow[0] * k.keyStep, -arrow[1] * k.keyStep, size); }
    else return null;
    return to;
  }

  // The view a box from `a` to `b` zooms to; null for a box under a point
  // both ways.
  function boxed(v, a, b, size) {
    if (Math.abs(a[0] - b[0]) < 1 && Math.abs(a[1] - b[1]) < 1) return null;
    var low = toItems(v, [Math.min(a[0], b[0]), Math.min(a[1], b[1])], size);
    var high = toItems(v, [Math.max(a[0], b[0]), Math.max(a[1], b[1])], size);
    var to = copy(v);
    fit(to, [low[0], low[1], high[0] - low[0], high[1] - low[1]], size);
    return to;
  }

  // The view `share` of the way from `from` to `to`, as ViewSpec::toward
  // gives it: the zoom moves by equal factors, and the point both views
  // put at one place on the canvas stays there.
  function toward(from, to, share, size) {
    var a = centerIn(from, size), b = centerIn(to, size);
    var ratio = Math.log(to.zoom / from.zoom);
    var weight = Math.abs(ratio) < 1e-12 ? share : Math.expm1(-share * ratio) / Math.expm1(-ratio);
    var out = copy(to);
    out.center = [a[0] + (b[0] - a[0]) * weight, a[1] + (b[1] - a[1]) * weight];
    out.zoom = from.zoom * Math.exp(share * ratio);
    return out;
  }

  // A drag's velocity at its release, in points a second, from its places
  // [[seconds, [x, y]], ...] oldest first, over the last `window` seconds;
  // null from fewer than two such places or no time between them.
  function releaseVelocity(samples, released, window) {
    var recent = samples.filter(function (s) { return released - s[0] <= window; });
    if (recent.length < 2) return null;
    var first = recent[0], last = recent[recent.length - 1], span = last[0] - first[0];
    if (!(span > 0)) return null;
    return [(last[1][0] - first[1][0]) / span, (last[1][1] - first[1][1]) / span];
  }

  // The glide after a pan let go at `velocity`, as Leaflet's inertia goes:
  // { by: [across, down], seconds }, or null when it would go less than
  // half a point either way.
  function glide(velocity, k) {
    var speed = Math.hypot(velocity[0], velocity[1]);
    if (!isFinite(speed) || speed <= 0) return null;
    var seconds = speed / k.glideDeceleration;
    var reach = k.glideLinearity * speed * seconds / 2;
    var by = [velocity[0] / speed * reach, velocity[1] / speed * reach];
    if (Math.abs(by[0]) < 0.5 && Math.abs(by[1]) < 0.5) return null;
    return { by: by, seconds: seconds };
  }

  // A gesture as ViewGesture::written writes it: Shift, Ctrl, Alt and
  // Space, joined by + to the button, Wheel, Pinch, a double-click or a
  // key; Shift is left out for Plus, Minus and Equals, which keyboards
  // type with it or without.
  function written(last, held) {
    var keepsShift = last !== 'Plus' && last !== 'Minus' && last !== 'Equals';
    var parts = [];
    if (held.shift && keepsShift) parts.push('Shift');
    if (held.ctrl) parts.push('Ctrl');
    if (held.alt) parts.push('Alt');
    if (held.space) parts.push('Space');
    parts.push(last);
    return parts.join('+');
  }

  // The move the view's controls bind to a gesture written as `written`,
  // or null.
  function moveFor(v, gesture) {
    return v && v.bindings && Object.prototype.hasOwnProperty.call(v.bindings, gesture) ? v.bindings[gesture] : null;
  }

  // The pointer as egui's input state stores it for a window (egui
  // input_state.rs, PointerState): the buttons down, Left, Right and
  // Middle, where and when the latest press began, whether the pointer has
  // since gone farther from there than a click goes, and the last two
  // clicks' times and the last one's place, at times in seconds and places
  // in points. A press is a click while it goes 6 points at most and lasts
  // 0.8 seconds at most; a click is double within 0.3 seconds of the click
  // before it and triple within 0.6 of the one before that, each within 6
  // points of the last click's place.
  var CLICK_DISTANCE = 6, CLICK_SECONDS = 0.8, DOUBLE_SECONDS = 0.3;
  function pointer() { return { down: [false, false, false], origin: null, began: null, far: false, lastClick: -Infinity, lastLastClick: -Infinity, lastClickAt: null }; }
  function pressed(p, slot, at, t) {
    p.origin = [at[0], at[1]];
    p.began = t;
    p.far = false;
    p.down[slot] = true;
  }
  function moved(p, at) {
    if (p.origin) p.far = p.far || Math.hypot(at[0] - p.origin[0], at[1] - p.origin[1]) > CLICK_DISTANCE;
  }
  // Whether a press down at `t` can still be let go as a click.
  function clickable(p, t) { return !p.far && !(p.began !== null && t - p.began > CLICK_SECONDS); }
  // The button in `slot` let go at `at` and `t`: its click, { count }, or
  // null.
  function released(p, slot, at, t) {
    var click = null;
    if (p.down.some(Boolean) && clickable(p, t)) {
      var near = !p.lastClickAt || Math.pow(at[0] - p.lastClickAt[0], 2) + Math.pow(at[1] - p.lastClickAt[1], 2) < CLICK_DISTANCE * CLICK_DISTANCE;
      var count = near && t - p.lastLastClick < 2 * DOUBLE_SECONDS ? 3 : near && t - p.lastClick < DOUBLE_SECONDS ? 2 : 1;
      p.lastLastClick = p.lastClick;
      p.lastClick = t;
      p.lastClickAt = [at[0], at[1]];
      click = { count: count };
    }
    p.origin = null;
    p.began = null;
    p.down[slot] = false;
    return click;
  }

  // egui's interaction with one widget over a frame (egui interaction.rs,
  // interact). `w` is the widget's { potentialClick, potentialDrag,
  // dragging }, and `p` the pointer once the frame's events are in. `frame`
  // is { events, escape, inside, hitClick, hitDrag, click, drag }: the
  // frame's presses { press: slot } and releases { release: slot, click } in
  // order, and whether Escape was pressed. It also says whether the pointer
  // is then inside the widget, whether egui's hit test there (hitTest) gives
  // a press to the widget as a click and as a drag, and whether the widget
  // senses clicks and drags. A press while no press is pending is the press
  // a click or a drag of the widget may come of, as the hit test gives it.
  // A release makes the widget clicked where the press could still be a
  // click, and ends its drag, as Escape does. Once the events are in, a drag
  // starts where the press is decidedly one, past a click's distance or
  // time. It also starts where the pointer is not inside the widget and the
  // hit test no longer gives it a click there, since a click is a release on
  // it. A widget sensing drags alone drags from the press. Returns whether
  // the widget was clicked, its click being each release in `frame.events`
  // that has one, and whether its drag started and stopped.
  function interact(w, p, frame, t) {
    var was = w.dragging, clicked = false, anyPressed = false, anyReleased = false, anyClick = false;
    if (frame.escape) {
      w.dragging = false;
      w.potentialDrag = false;
    }
    frame.events.forEach(function (e) {
      if (e.press !== undefined) {
        anyPressed = true;
        if (!w.potentialClick) w.potentialClick = frame.hitClick;
        if (!w.potentialDrag) w.potentialDrag = frame.hitDrag;
      } else if (e.release !== undefined) {
        anyReleased = true;
        if (e.click) anyClick = true;
        if (e.click && w.potentialClick) clicked = true;
        w.potentialClick = false;
        w.potentialDrag = false;
        w.dragging = false;
      }
    });
    var down = p.down.some(Boolean), couldClick = (down || anyReleased) && clickable(p, t);
    var decided = (down || anyReleased) && !anyPressed && !couldClick && !anyClick;
    if (!w.dragging && w.potentialDrag && (!frame.click || decided || !(frame.inside || frame.hitClick))) w.dragging = true;
    if (!couldClick) w.potentialClick = false;
    if (!down) {
      w.potentialClick = false;
      w.potentialDrag = false;
    }
    return { clicked: clicked, started: w.dragging && !was, stopped: was && !w.dragging };
  }

  // egui's interact radius (Style::interaction.interact_radius): how far
  // from a widget, in points, a press still reaches it.
  var INTERACT_RADIUS = 5;
  // A box [left, top, right, bottom] as egui's Rect is one: negative where
  // an edge passes its opposite; whether it holds a point or another box,
  // edges included; and how far a point lies from it, squared, nothing
  // inside it and every point from a negative box.
  function negative(r) { return r[2] < r[0] || r[3] < r[1]; }
  function holds(r, at) { return r[0] <= at[0] && at[0] <= r[2] && r[1] <= at[1] && at[1] <= r[3]; }
  function holdsBox(r, o) { return holds(r, [o[0], o[1]]) && holds(r, [o[2], o[3]]); }
  function distanceSq(r, at) {
    if (negative(r)) return Infinity;
    var dx = r[0] > at[0] ? r[0] - at[0] : at[0] > r[2] ? at[0] - r[2] : 0;
    var dy = r[1] > at[1] ? r[1] - at[1] : at[1] > r[3] ? at[1] - r[3] : 0;
    return dx * dx + dy * dy;
  }
  // The widget of `widgets` nearest `at`, within `most` points, the last of
  // a tie, as egui's find_closest_within finds it; null for none.
  function nearest(widgets, at, most) {
    var found = null, best = most * most;
    widgets.forEach(function (w) {
      if (negative(w.rect)) return;
      var d = distanceSq(w.rect, at);
      if (d <= best) {
        best = d;
        found = w;
      }
    });
    return found;
  }
  // The widgets of `close`, back to front, that a press at `at` would click
  // and drag, as egui's hit_test_on_close picks them. Each is the topmost
  // under the pointer that senses it, and where none is, the nearest that
  // senses either, taking what it senses. Under a widget sensing drags
  // alone, a nearer click widget inside it takes the click, and the drag too
  // when it senses both. A nearer click widget beside it takes the click and
  // leaves the drag to the widget under the pointer. With a drag widget
  // inside it and no click widget near, that one takes the drag. Under both,
  // the topmost decides: one sensing both takes both, a click widget on top
  // leaves the drag to the one under it, and a drag widget on top takes no
  // click.
  function pick(close, at) {
    var clicks = close.filter(function (w) { return w.click; }), drags = close.filter(function (w) { return w.drag; });
    var hitClick = nearest(clicks, at, 0), hitDrag = nearest(drags, at, 0);
    var made = function (click, drag) { return { click: click, drag: drag }; };
    if (!hitClick && !hitDrag) {
      var any = nearest(close.filter(function (w) { return w.click || w.drag; }), at, Infinity);
      return made(any && any.click ? any : null, any && any.drag ? any : null);
    }
    if (!hitClick) {
      var nearClick = nearest(clicks, at, Infinity);
      if (nearClick) {
        var within = holdsBox(hitDrag.rect, nearClick.rect);
        if (nearClick.drag) return within ? made(nearClick, nearClick) : made(null, hitDrag);
        return within ? made(nearClick, hitDrag) : made(null, hitDrag);
      }
      var nearDrag = nearest(drags.filter(function (w) { return w.id !== hitDrag.id; }), at, Infinity);
      return nearDrag && holdsBox(hitDrag.rect, nearDrag.rect) ? made(null, nearDrag) : made(null, hitDrag);
    }
    if (!hitDrag) return made(hitClick, null);
    var place = function (w) { for (var i = 0; i < close.length; i++) if (close[i].id === w.id) return i; return -1; };
    if (place(hitDrag) < place(hitClick)) return hitClick.drag ? made(hitClick, hitClick) : made(hitClick, hitDrag);
    return hitDrag.click ? made(hitDrag, hitDrag) : made(null, hitDrag);
  }
  // egui's hit test (egui hit_test.rs, hit_test) at `at`, within `radius`
  // points: `widgets` back to front, each { id, layer, rect: [left, top,
  // right, bottom], click, drag, enabled }, a higher layer drawn over a
  // lower one, the widgets of a layer in the order given. The widgets
  // within the radius count, from the top down through the layers until a
  // widget covers the whole radius around the pointer, its layer the last
  // that counts; a disabled one senses nothing, and one inside a widget of
  // another layer drawn over it is hidden. Returns the ids of the widgets
  // that count (`close`) and of those holding the pointer (`contains`),
  // back to front, and the ids of the widgets a press there would click and
  // drag (pick), or null.
  function hitTest(widgets, at, radius) {
    var ordered = widgets.map(function (w, i) { return [w, i]; }).sort(function (a, b) { return a[0].layer - b[0].layer || a[1] - b[1]; });
    var close = ordered.map(function (o) { return o[0]; }).filter(function (w) { return !negative(w.rect) && distanceSq(w.rect, at) <= radius * radius; });
    var layers = {};
    for (var i = close.length - 1; i >= 0; i--) {
      var r = close[i].rect;
      layers[close[i].layer] = true;
      if (holds([r[0] + radius, r[1] + radius, r[2] - radius, r[3] - radius], at)) break;
    }
    close = close.filter(function (w) { return layers[w.layer]; }).map(function (w) {
      return w.enabled === false ? { id: w.id, layer: w.layer, rect: w.rect, click: false, drag: false, enabled: false } : w;
    });
    var hidden = close.map(function (w, j) {
      return close.slice(j + 1).some(function (over) { return over.layer !== w.layer && holdsBox(over.rect, w.rect); });
    });
    close = close.filter(function (w, j) { return !hidden[j]; });
    var picked = pick(close, at);
    var ids = function (list) { return list.map(function (w) { return w.id; }); };
    return { close: ids(close), contains: ids(close.filter(function (w) { return holds(w.rect, at); })), click: picked.click ? picked.click.id : null, drag: picked.drag ? picked.drag.id : null };
  }
  // The release of the button in `slot` among a frame's `events` whose
  // click a widget took, `count` clicks long when given; null for none.
  function clickBy(result, events, slot, count) {
    if (!result.clicked) return null;
    for (var i = 0; i < events.length; i++) {
      var e = events[i];
      if (e.release === slot && e.click && (count === undefined || e.click.count === count)) return e;
    }
    return null;
  }

  // A number rounded to the nearest whole one, halves away from 0, as Rust
  // rounds.
  function roundAway(x) { return x < 0 ? -Math.round(-x) : Math.round(x); }

  // A color written #rrggbbaa as its four bytes, and back.
  function bytesOf(hex) { return [1, 3, 5, 7].map(function (i) { return parseInt(hex.slice(i, i + 2), 16); }); }
  function hexOf(c) { return '#' + c.map(function (n) { return ('0' + n.toString(16)).slice(-2); }).join(''); }

  // The value `share` of the way from `a` to `b`, a number, an x,y pair or
  // a color, as Target::toward gives it: numbers and pairs straight, colors
  // premultiplied, each channel cut to a byte; values of two kinds
  // give `b`.
  function mix(a, b, share) {
    function m(x, y) { return x + (y - x) * share; }
    if (typeof a === 'number' && typeof b === 'number') return m(a, b);
    if (Array.isArray(a) && Array.isArray(b)) return [m(a[0], b[0]), m(a[1], b[1])];
    if (typeof a === 'string' && typeof b === 'string') {
      var ca = bytesOf(a), cb = bytesOf(b), aa = ca[3] / 255, ba = cb[3] / 255;
      var alpha = clamp(m(aa, ba), 0, 1);
      var channel = function (i) { return alpha > 0 ? clamp(roundAway(m(ca[i] * aa, cb[i] * ba) / alpha), 0, 255) : 0; };
      return hexOf([channel(0), channel(1), channel(2), roundAway(alpha * 255)]);
    }
    return b;
  }

  // An animation's passes together, in seconds, as Animation::length gives
  // them: null for one that repeats forever.
  function passOf(a) { return a.reverse ? 2 * a.duration : a.duration; }
  function lengthOf(a) { return a.repeat === 'Forever' ? null : passOf(a) * a.repeat; }

  // How far along the way from its start to its To an animation's property
  // is `elapsed` seconds after it began, as Animation::share gives it.
  function share(a, elapsed) {
    elapsed = Math.max(0, elapsed);
    var l = lengthOf(a);
    if (l !== null && elapsed >= l) return a.reverse ? 0 : 1;
    var into = elapsed % passOf(a);
    return ease(a.easing, into <= a.duration ? into / a.duration : 1 - (into - a.duration) / a.duration);
  }

  // A running animation { animation, began, from }: its value at `now`,
  // null while its start is not yet read, and whether it still moves.
  function motionValue(m, now) { return m.from === null || m.from === undefined ? null : mix(m.from, m.animation.to, share(m.animation, now - m.began)); }
  function motionMoving(m, now) { var l = lengthOf(m.animation); return l === null || Math.max(0, now - m.began) < l; }

  var TEXT_SIZE = 14;
  var CLEAR = '#00000000';
  var BOXED = ['Rect', 'RoundedRect', 'Ellipse', 'Image'];

  // An item's own value of `property`, by its lowercase name, as
  // Item::value_of reads it: the default where the item gives none, a Fill
  // or Stroke it leaves to its group read as clear; null for a property
  // its kind does not move, or a Fill that is a gradient.
  function valueOf(item, property) {
    var s = item.style, sh = item.shape;
    switch (property) {
      case 'fill': return !s.fill ? CLEAR : s.fill.color !== undefined ? s.fill.color : null;
      case 'stroke': return s.stroke || CLEAR;
      case 'strokewidth': return s.strokeWidth === null || s.strokeWidth === undefined ? 1 : s.strokeWidth;
      case 'opacity': return s.opacity;
      case 'rotate': return s.rotate;
      case 'scale': return s.scale.slice();
      case 'skew': return s.skew.slice();
      case 'origin': return s.origin ? s.origin.slice() : [0.5, 0.5];
    }
    if (BOXED.indexOf(sh.kind) >= 0 && ['x', 'y', 'width', 'height'].indexOf(property) >= 0) return sh[property];
    if (sh.kind === 'RoundedRect' && property === 'radius') return sh.radius;
    if (sh.kind === 'Line' && ['x1', 'y1', 'x2', 'y2'].indexOf(property) >= 0) return sh[property];
    if ((sh.kind === 'Text' || sh.kind === 'Group') && (property === 'x' || property === 'y')) return sh[property];
    if (sh.kind === 'Text' && property === 'size') return sh.size === null || sh.size === undefined ? TEXT_SIZE : sh.size;
    return null;
  }

  // Sets the drawn `property` of `item`, a copy whose shape and style are
  // its own, to `value`, as Item::set_value sets it: a value of the wrong
  // kind and a property the kind does not move change nothing, and a size,
  // width or opacity an easing moves past its bounds is set to the bound it
  // passed.
  function setValue(item, property, value) {
    var s = item.style, sh = item.shape, num = typeof value === 'number', pair = Array.isArray(value), color = typeof value === 'string';
    if (property === 'fill') { if (color) s.fill = { color: value }; return; }
    if (property === 'stroke') { if (color) s.stroke = value; return; }
    if (property === 'strokewidth') { if (num) s.strokeWidth = Math.max(0, value); return; }
    if (property === 'opacity') { if (num) s.opacity = clamp(value, 0, 1); return; }
    if (property === 'rotate') { if (num) s.rotate = value; return; }
    if (property === 'scale') { if (pair) s.scale = value.slice(); return; }
    if (property === 'skew') { if (pair) s.skew = [clamp(value[0], -89.9, 89.9), clamp(value[1], -89.9, 89.9)]; return; }
    if (property === 'origin') { if (pair) s.origin = value.slice(); return; }
    if (!num) return;
    if (BOXED.indexOf(sh.kind) >= 0 && (property === 'x' || property === 'y')) sh[property] = value;
    else if (BOXED.indexOf(sh.kind) >= 0 && (property === 'width' || property === 'height')) sh[property] = Math.max(0, value);
    else if (sh.kind === 'RoundedRect' && property === 'radius') sh.radius = Math.max(0, value);
    else if (sh.kind === 'Line' && ['x1', 'y1', 'x2', 'y2'].indexOf(property) >= 0) sh[property] = value;
    else if ((sh.kind === 'Text' || sh.kind === 'Group') && (property === 'x' || property === 'y')) sh[property] = value;
    else if (sh.kind === 'Text' && property === 'size') sh.size = Math.max(value, 2.2250738585072014e-308);
  }

  // What a group passes on to its items, as SVG's <g>: its own fill,
  // stroke, stroke width, dash, caps and joins over those passed to it.
  var NOTHING_PASSED = { fill: null, stroke: null, strokeWidth: null, dash: null, cap: null, join: null };
  function under(passed, style) {
    function pick(own, given) { return own !== null && own !== undefined ? own : given; }
    return { fill: pick(style.fill, passed.fill), stroke: pick(style.stroke, passed.stroke), strokeWidth: pick(style.strokeWidth, passed.strokeWidth), dash: pick(style.dash, passed.dash), cap: pick(style.cap, passed.cap), join: pick(style.join, passed.join) };
  }

  // Where an animation of `property` begins on `item` under what its groups
  // pass on when no item gives the property, as egui's first_drawn finds
  // it: a Text giving neither Fill nor Stroke filled in the theme's `text`
  // color, any other item outlined in it, else a missing color clear and a
  // missing width 1.
  function firstDrawn(item, passedDown, property, text) {
    var passed = under(passedDown, item.style);
    var neither = !passed.fill && !passed.stroke, isText = item.shape.kind === 'Text';
    if (property === 'fill') return neither && isText ? text : passed.fill && passed.fill.color !== undefined ? passed.fill.color : CLEAR;
    if (property === 'stroke') return !passed.stroke && ((neither && !isText) || item.shape.kind === 'Line') ? text : passed.stroke || CLEAR;
    if (property === 'strokewidth') return passed.strokeWidth === null ? 1 : passed.strokeWidth;
    return valueOf(item, property);
  }

  // What fills and strokes `item` under `passed`, as egui's paints gives
  // them: a Line is never filled; a Text giving neither is filled in the
  // theme's `text` color, and any other item giving neither outlined in it.
  function paints(item, passed, text) {
    var k = item.shape.kind;
    if (k === 'Line') return { fill: null, stroke: passed.stroke || text };
    if (!passed.fill && !passed.stroke) return k === 'Text' ? { fill: { color: text }, stroke: null } : { fill: null, stroke: text };
    return { fill: passed.fill, stroke: passed.stroke };
  }

  // A canvas's items stored for change, from a scene's nested items: each by
  // its id with its group's id, a Group's items as ids.
  function boardOf(items) {
    var board = { items: {}, top: [] };
    function put(item, parent) {
      var kept = { id: item.id, name: item.name, shape: item.shape, style: item.style, animations: item.animations || [], children: [], parent: parent };
      board.items[item.id] = kept;
      (item.children || []).forEach(function (c) { kept.children.push(put(c, item.id)); });
      return item.id;
    }
    items.forEach(function (i) { board.top.push(put(i, null)); });
    return board;
  }

  // A canvas's animations, by item id and then by property, each
  // { animation, began, from }, as Motions stores them: started as items
  // come with an Animate, ended by a direct set or an Animate of null, a
  // new Animate replacing only those of its properties.
  function started(motions, board, ids, now) {
    ids.forEach(function (id) {
      var e = board.items[id];
      if (e) e.animations.forEach(function (a) { startOne(motions, board, id, a, now); });
    });
  }
  function changed(motions, board, id, set, animate, now) {
    var own = motions[id];
    if (own) set.forEach(function (k) { delete own[k]; });
    if (animate === 'clear') delete motions[id];
    else if (animate === 'start') started(motions, board, [id], now);
  }
  function startOne(motions, board, id, a, now) {
    var from = drawn(motions, board, id, a.property, now);
    (motions[id] || (motions[id] = {}))[a.property] = { animation: a, began: now, from: from };
  }
  function forget(motions, ids) { ids.forEach(function (id) { delete motions[id]; }); }
  // The value of `property` drawn on item `id` at `now`: its animation's,
  // else its own, else for a Fill, Stroke or StrokeWidth it leaves to its
  // groups the nearest group's; null where only the theme sets it.
  function drawn(motions, board, id, property, now) {
    var m = motions[id] && motions[id][property];
    var v = m ? motionValue(m, now) : null;
    if (v !== null) return v;
    var e = board.items[id];
    if (!e) return null;
    var s = e.style;
    var left = property === 'fill' ? !s.fill : property === 'stroke' ? !s.stroke : property === 'strokewidth' ? s.strokeWidth === null || s.strokeWidth === undefined : false;
    if (!left) return valueOf(e, property);
    return e.parent === null ? null : drawn(motions, board, e.parent, property, now);
  }
  function moving(motions, now) {
    return Object.keys(motions).some(function (id) { return Object.keys(motions[id]).some(function (p) { return motionMoving(motions[id][p], now); }); });
  }

  // The outline a shape draws, as Shape::outline gives it: segments in its
  // own coordinates, each subpath turning clockwise as y grows downwards;
  // null for Text, an Image and a Group.
  function outline(sh) {
    var k = sh.kind;
    function rect(x, y, w, h) { return [['M', x, y], ['L', x + w, y], ['L', x + w, y + h], ['L', x, y + h], ['Z']]; }
    if (k === 'Rect') return rect(sh.x, sh.y, sh.width, sh.height);
    if (k === 'RoundedRect') {
      var r = Math.min(sh.radius, sh.width / 2, sh.height / 2);
      if (!(r > 0)) return rect(sh.x, sh.y, sh.width, sh.height);
      var right = sh.x + sh.width, bottom = sh.y + sh.height;
      var corner = function (x, y) { return ['A', r, r, 0, false, true, x, y]; };
      return [['M', sh.x, sh.y + r], corner(sh.x + r, sh.y), ['L', right - r, sh.y], corner(right, sh.y + r), ['L', right, bottom - r], corner(right - r, bottom), ['L', sh.x + r, bottom], corner(sh.x, bottom - r), ['Z']];
    }
    if (k === 'Ellipse') {
      if (sh.width === 0 || sh.height === 0) return [['M', sh.x, sh.y], ['L', sh.x + sh.width, sh.y + sh.height], ['Z']];
      var cy = sh.y + sh.height / 2;
      var half = function (x) { return ['A', sh.width / 2, sh.height / 2, 0, false, true, x, cy]; };
      return [['M', sh.x + sh.width, cy], half(sh.x), half(sh.x + sh.width), ['Z']];
    }
    if (k === 'Line') return [['M', sh.x1, sh.y1], ['L', sh.x2, sh.y2]];
    if (k === 'Polyline' || k === 'Polygon') {
      var out = [];
      for (var i = 0; i + 1 < sh.points.length; i += 2) out.push([i === 0 ? 'M' : 'L', sh.points[i], sh.points[i + 1]]);
      if (k === 'Polygon') out.push(['Z']);
      return out;
    }
    if (k === 'Path') return sh.segments;
    return null;
  }

  // Segments as SVG path data, which Path2D reads.
  function pathData(segments) {
    return segments.map(function (s) { return s[0] === 'A' ? ['A', s[1], s[2], s[3], s[4] ? 1 : 0, s[5] ? 1 : 0, s[6], s[7]].join(' ') : s.join(' '); }).join(' ');
  }

  // The subpaths of `segments` that end in Z, in their order, each starting
  // with its M, a subpath begun after a Z without one moving from where the
  // Z ended, as svg_path::closed gives them: the part of an outline with an
  // inside whether or not it is filled.
  function closed(segments) {
    var out = [], open = [], current = [0, 0], start = [0, 0];
    segments.forEach(function (s) {
      if (s[0] === 'M') {
        open = [s];
        current = [s[1], s[2]];
        start = current;
      } else if (s[0] === 'Z') {
        if (open.length) out = out.concat(open, [s]);
        open = [];
        current = start;
      } else {
        if (!open.length) {
          open.push(['M', current[0], current[1]]);
          start = current;
        }
        open.push(s);
        current = [s[s.length - 2], s[s.length - 1]];
      }
    });
    return out;
  }

  // The arc an Arc segment draws from `from` to `to` in center form
  // { center, radii, rotation (radians), start, sweep }, as center_arc finds
  // it; null for a straight line, a radius being zero, or nothing, the ends
  // being one point.
  function centerArc(from, radii, rotation, large, sweep, to) {
    var rx = Math.abs(radii[0]), ry = Math.abs(radii[1]);
    if ((from[0] === to[0] && from[1] === to[1]) || rx === 0 || ry === 0) return null;
    var phi = rotation * (PI / 180), s = Math.sin(phi), c = Math.cos(phi);
    var dx = (from[0] - to[0]) / 2, dy = (from[1] - to[1]) / 2;
    var x1 = c * dx + s * dy, y1 = -s * dx + c * dy;
    var reach = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if (reach > 1) { rx *= Math.sqrt(reach); ry *= Math.sqrt(reach); }
    var rx2 = rx * rx, ry2 = ry * ry;
    var portion = Math.sqrt(Math.max(0, (rx2 * ry2 - rx2 * y1 * y1 - ry2 * x1 * x1) / (rx2 * y1 * y1 + ry2 * x1 * x1)));
    if (large === sweep) portion = -portion;
    var cx1 = portion * rx * y1 / ry, cy1 = -portion * ry * x1 / rx;
    var center = [c * cx1 - s * cy1 + (from[0] + to[0]) / 2, s * cx1 + c * cy1 + (from[1] + to[1]) / 2];
    function angle(ux, uy, vx, vy) { return Math.atan2(ux * vy - uy * vx, ux * vx + uy * vy); }
    var ux = (x1 - cx1) / rx, uy = (y1 - cy1) / ry, vx = (-x1 - cx1) / rx, vy = (-y1 - cy1) / ry;
    var turn = angle(ux, uy, vx, vy);
    if (!sweep && turn > 0) turn -= 2 * PI; else if (sweep && turn < 0) turn += 2 * PI;
    return { center: center, radii: [rx, ry], rotation: phi, start: angle(1, 0, ux, uy), sweep: turn };
  }
  function arcAt(arc, a) {
    var s = Math.sin(arc.rotation), c = Math.cos(arc.rotation), x = arc.radii[0] * Math.cos(a), y = arc.radii[1] * Math.sin(a);
    return [arc.center[0] + c * x - s * y, arc.center[1] + s * x + c * y];
  }
  function arcPasses(arc, a) {
    var tau = 2 * PI, rem = function (v) { return ((v % tau) + tau) % tau; };
    return arc.sweep >= 0 ? rem(a - arc.start) <= arc.sweep : rem(arc.start - a) <= -arc.sweep;
  }

  // `p` placed by the plane transform [a, b, c, d, e, f].
  function applied(m, p) { return [m[0] * p[0] + m[2] * p[1] + m[4], m[1] * p[0] + m[3] * p[1] + m[5]]; }

  // The smallest box [x, y, width, height] around what `segments` draw
  // once transform `m` places them, as svg_path::bounds finds it; null
  // when they draw nothing.
  function bounds(segments, m) {
    var low = [Infinity, Infinity], high = [-Infinity, -Infinity];
    function take(p) { low = [Math.min(low[0], p[0]), Math.min(low[1], p[1])]; high = [Math.max(high[0], p[0]), Math.max(high[1], p[1])]; }
    function place(p) { return applied(m, p); }
    var current = [0, 0], start = [0, 0];
    segments.forEach(function (s) {
      var from = place(current);
      if (s[0] === 'M') { current = [s[1], s[2]]; start = current; return; }
      if (s[0] === 'L') { take(from); take(place([s[1], s[2]])); current = [s[1], s[2]]; return; }
      if (s[0] === 'Q') {
        var q1 = place([s[1], s[2]]), q2 = place([s[3], s[4]]);
        take(from); take(q2);
        for (var axis = 0; axis < 2; axis++) {
          var bend = from[axis] - 2 * q1[axis] + q2[axis];
          if (bend !== 0) { var t = (from[axis] - q1[axis]) / bend; if (t > 0 && t < 1) take(quadAt(from, q1, q2, t)); }
        }
        current = [s[3], s[4]];
        return;
      }
      if (s[0] === 'C') {
        var c1 = place([s[1], s[2]]), c2 = place([s[3], s[4]]), c3 = place([s[5], s[6]]);
        take(from); take(c3);
        for (var ax = 0; ax < 2; ax++) cubicTurns(from[ax], c1[ax], c2[ax], c3[ax]).forEach(function (t) { take(cubicAt(from, c1, c2, c3, t)); });
        current = [s[5], s[6]];
        return;
      }
      if (s[0] === 'A') {
        var to = [s[6], s[7]];
        if (current[0] === to[0] && current[1] === to[1]) return;
        take(from); take(place(to));
        var arc = centerArc(current, [s[1], s[2]], s[3], s[4], s[5], to);
        if (arc) {
          var sn = Math.sin(arc.rotation), co = Math.cos(arc.rotation), rx = arc.radii[0], ry = arc.radii[1];
          [[(m[0] * co + m[2] * sn) * rx, (-m[0] * sn + m[2] * co) * ry], [(m[1] * co + m[3] * sn) * rx, (-m[1] * sn + m[3] * co) * ry]].forEach(function (uv) {
            var turn = Math.atan2(uv[1], uv[0]);
            [turn, turn + PI].forEach(function (a) { if (arcPasses(arc, a)) take(place(arcAt(arc, a))); });
          });
        }
        current = to;
        return;
      }
      if (s[0] === 'Z') { take(from); take(place(start)); current = start; }
    });
    return low[0] <= high[0] ? [low[0], low[1], high[0] - low[0], high[1] - low[1]] : null;
  }
  function quadAt(p0, p1, p2, t) { var u = 1 - t; return [u * u * p0[0] + 2 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2 * u * t * p1[1] + t * t * p2[1]]; }
  function cubicAt(p0, p1, p2, p3, t) {
    var u = 1 - t;
    function at(i) { return u * u * u * p0[i] + 3 * u * u * t * p1[i] + 3 * u * t * t * p2[i] + t * t * t * p3[i]; }
    return [at(0), at(1)];
  }
  function cubicTurns(p0, p1, p2, p3) {
    var a = -p0 + 3 * p1 - 3 * p2 + p3, b = 2 * (p0 - 2 * p1 + p2), c = p1 - p0, roots;
    if (Math.abs(a) < 1e-12 * Math.max(Math.abs(b) + Math.abs(c), 1e-300)) roots = b === 0 ? [] : [-c / b];
    else {
      var disc = b * b - 4 * a * c;
      roots = disc < 0 ? [] : [(-b + Math.sqrt(disc)) / (2 * a), (-b - Math.sqrt(disc)) / (2 * a)];
    }
    return roots.filter(function (t) { return t > 0 && t < 1; });
  }

  // The box a fit counts for an outline `full` draws: its exact box grown
  // on each side by half of `stroke`, the stroke's width when it is
  // stroked, as `full` stretches it across and down; null when it draws
  // nothing.
  function visualBox(segments, full, stroke) {
    var b = bounds(segments, full);
    if (!b) return null;
    var across = stroke > 0 ? stroke / 2 * Math.hypot(full[0], full[2]) : 0, down = stroke > 0 ? stroke / 2 * Math.hypot(full[1], full[3]) : 0;
    return [b[0] - across, b[1] - down, b[2] + 2 * across, b[3] + 2 * down];
  }

  // The stops a Canvas 2D gradient takes to blend `colors`, #rrggbbaa, at
  // `stops` as egui blends them, premultiplied: where two stops' alphas
  // differ, fifteen more between them, each the premultiplied blend
  // unpremultiplied, and a clear stop taking on each side the color it
  // blends toward there; each [offset, #rrggbbaa].
  var BETWEEN = 16;
  function gradientStops(colors, stops) {
    var bytes = colors.map(bytesOf);
    var clearOf = function (c) { return hexOf([c[0], c[1], c[2], 0]); };
    var out = [];
    for (var i = 0; i < bytes.length; i++) {
      var own = bytes[i], left = i > 0 ? bytes[i - 1] : own, right = i + 1 < bytes.length ? bytes[i + 1] : own;
      if (own[3] > 0) out.push([stops[i], hexOf(own)]);
      else {
        out.push([stops[i], clearOf(left[3] > 0 ? left : right)]);
        if (left[3] > 0 && right[3] > 0) out.push([stops[i], clearOf(right)]);
      }
      if (i + 1 < bytes.length && bytes[i][3] !== bytes[i + 1][3]) {
        var a = bytes[i], b = bytes[i + 1], aa = a[3] / 255, ba = b[3] / 255;
        for (var j = 1; j < BETWEEN; j++) {
          var t = j / BETWEEN, alpha = aa + (ba - aa) * t;
          var ch = function (k) { return clamp(Math.round((a[k] * aa + (b[k] * ba - a[k] * aa) * t) / alpha), 0, 255); };
          out.push([stops[i] + (stops[i + 1] - stops[i]) * t, hexOf([ch(0), ch(1), ch(2), Math.round(alpha * 255)])]);
        }
      }
    }
    return out;
  }

  // The height of a row of text `size` points tall and where its baseline
  // lies below the row's top, in points, in the face whose metrics `face`
  // gives in its own units, as egui lays one row out: the ascent, descent
  // and line gap each scaled and rounded to 1/32 of a point, the row and
  // the baseline each rounded to a whole physical pixel at `ppp` pixels a
  // point, reckoned in single precision as egui reckons them.
  function textRow(face, size, ppp) {
    var f = Math.fround, scale = f(f(size) / f(face.units));
    var round = function (v) { return f(roundAway(f(v / 0.03125)) * 0.03125); };
    var ascent = round(f(face.ascent * scale)), descent = round(f(face.descent * scale)), gap = round(f(face.gap * scale));
    var pixel = function (v) { return f(roundAway(f(v * f(ppp))) / f(ppp)); };
    return { height: pixel(f(f(ascent - descent) + gap)), baseline: pixel(ascent) };
  }

  // The rows of a Text's `text` as egui stacks them, one for each of its
  // lines, `row` the row textRow gives at `ppp` pixels a point: each line
  // with its baseline's offset below the text's top, and the rows' height
  // together, each row's top the one before it added to its height and
  // rounded to a whole physical pixel, in single precision as egui does.
  function textLines(text, row, ppp) {
    var f = Math.fround, top = 0;
    var pixel = function (v) { return f(roundAway(f(v * f(ppp))) / f(ppp)); };
    var lines = String(text).split('\n').map(function (line) {
      var at = { text: line, baseline: f(top + row.baseline) };
      top = pixel(f(top + row.height));
      return at;
    });
    return { lines: lines, height: top };
  }

  return {
    mix: mix,
    share: share,
    motionValue: motionValue,
    motionMoving: motionMoving,
    valueOf: valueOf,
    setValue: setValue,
    under: under,
    nothingPassed: NOTHING_PASSED,
    firstDrawn: firstDrawn,
    paints: paints,
    boardOf: boardOf,
    started: started,
    changed: changed,
    forget: forget,
    drawn: drawn,
    moving: moving,
    outline: outline,
    pathData: pathData,
    closed: closed,
    centerArc: centerArc,
    applied: applied,
    bounds: bounds,
    visualBox: visualBox,
    gradientStops: gradientStops,
    textRow: textRow,
    textLines: textLines,
    ease: ease,
    easings: Object.keys(curves),
    transform: transform,
    toItems: toItems,
    centerIn: centerIn,
    copy: copy,
    pan: pan,
    zoomAbout: zoomAbout,
    wheel: wheel,
    dragged: dragged,
    fit: fit,
    stepped: stepped,
    boxed: boxed,
    toward: toward,
    releaseVelocity: releaseVelocity,
    glide: glide,
    written: written,
    moveFor: moveFor,
    clickSeconds: CLICK_SECONDS,
    pointer: pointer,
    pressed: pressed,
    moved: moved,
    released: released,
    clickable: clickable,
    interact: interact,
    clickBy: clickBy,
    interactRadius: INTERACT_RADIUS,
    hitTest: hitTest
  };
})();
