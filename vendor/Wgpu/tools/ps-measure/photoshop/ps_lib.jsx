// Builds stroked shape layers in a new Photoshop document from plain case
// objects, saves the document as a PSD and closes it. Documents are 72 ppi,
// so a path point's coordinates in points are its pixel coordinates.
//
// A case: { name, fill: [r, g, b], subpaths: [{ closed, op, knots }], stroke }
// where op is add, subtract, intersect or exclude, each knot is
// [x, y, inX, inY, outX, outY] (anchor, the handle before it, the handle
// after it), and stroke is { enabled, fillEnabled, width, align, join, cap,
// miterLimit, dashes, dashOffset, opacity, color, strokeAdjust }.

function sid(s) { return stringIDToTypeID(s); }
function cid(s) { return charIDToTypeID(s); }

var ALIGN = { inside: "strokeStyleAlignInside", center: "strokeStyleAlignCenter", outside: "strokeStyleAlignOutside" };
var JOIN = { miter: "strokeStyleMiterJoin", round: "strokeStyleRoundJoin", bevel: "strokeStyleBevelJoin" };
var CAP = { butt: "strokeStyleButtCap", round: "strokeStyleRoundCap", square: "strokeStyleSquareCap" };
var OPERATION = { add: ShapeOperation.SHAPEADD, subtract: ShapeOperation.SHAPESUBTRACT, intersect: ShapeOperation.SHAPEINTERSECT, exclude: ShapeOperation.SHAPEXOR };

function rgbDescriptor(rgb) {
    var d = new ActionDescriptor();
    d.putDouble(cid("Rd  "), rgb[0]);
    d.putDouble(cid("Grn "), rgb[1]);
    d.putDouble(cid("Bl  "), rgb[2]);
    return d;
}

function pathOf(doc, name, subpaths) {
    var infos = [];
    for (var s = 0; s < subpaths.length; s++) {
        var points = [];
        var knots = subpaths[s].knots;
        for (var k = 0; k < knots.length; k++) {
            var n = knots[k];
            var p = new PathPointInfo();
            var corner = n[2] == n[0] && n[3] == n[1] && n[4] == n[0] && n[5] == n[1];
            p.kind = corner ? PointKind.CORNERPOINT : PointKind.SMOOTHPOINT;
            // The saved file's knot takes rightDirection as the handle before
            // the anchor and leftDirection as the one after it.
            p.anchor = [n[0], n[1]];
            p.rightDirection = [n[2], n[3]];
            p.leftDirection = [n[4], n[5]];
            points.push(p);
        }
        var info = new SubPathInfo();
        info.closed = subpaths[s].closed;
        info.operation = OPERATION[subpaths[s].op];
        info.entireSubPath = points;
        infos.push(info);
    }
    return doc.pathItems.add(name, infos);
}

// A solid color fill layer over the selected path, which Photoshop takes as
// the new layer's vector mask.
function fillLayerFromPath(path, rgb) {
    path.select();
    var color = new ActionDescriptor();
    color.putObject(cid("Clr "), cid("RGBC"), rgbDescriptor(rgb));
    var using = new ActionDescriptor();
    using.putObject(cid("Type"), sid("solidColorLayer"), color);
    var r = new ActionReference();
    r.putClass(sid("contentLayer"));
    var d = new ActionDescriptor();
    d.putReference(cid("null"), r);
    d.putObject(cid("Usng"), sid("contentLayer"), using);
    executeAction(cid("Mk  "), d, DialogModes.NO);
}

// Sets every field of the active shape layer's stroke, typed as Photoshop's
// own files store them.
function setStroke(s) {
    var style = new ActionDescriptor();
    style.putInteger(sid("strokeStyleVersion"), 2);
    style.putBoolean(sid("strokeEnabled"), s.enabled);
    style.putBoolean(sid("fillEnabled"), s.fillEnabled);
    style.putUnitDouble(sid("strokeStyleLineWidth"), cid("#Pxl"), s.width);
    style.putUnitDouble(sid("strokeStyleLineDashOffset"), cid("#Pnt"), s.dashOffset);
    style.putDouble(sid("strokeStyleMiterLimit"), s.miterLimit);
    style.putEnumerated(sid("strokeStyleLineCapType"), sid("strokeStyleLineCapType"), sid(CAP[s.cap]));
    style.putEnumerated(sid("strokeStyleLineJoinType"), sid("strokeStyleLineJoinType"), sid(JOIN[s.join]));
    style.putEnumerated(sid("strokeStyleLineAlignment"), sid("strokeStyleLineAlignment"), sid(ALIGN[s.align]));
    style.putBoolean(sid("strokeStyleScaleLock"), false);
    style.putBoolean(sid("strokeStyleStrokeAdjust"), s.strokeAdjust);
    var dashes = new ActionList();
    for (var i = 0; i < s.dashes.length; i++) {
        dashes.putUnitDouble(cid("#Nne"), s.dashes[i]);
    }
    style.putList(sid("strokeStyleLineDashSet"), dashes);
    style.putEnumerated(sid("strokeStyleBlendMode"), cid("BlnM"), cid("Nrml"));
    style.putUnitDouble(sid("strokeStyleOpacity"), cid("#Prc"), s.opacity);
    var content = new ActionDescriptor();
    content.putObject(cid("Clr "), cid("RGBC"), rgbDescriptor(s.color));
    style.putObject(sid("strokeStyleContent"), sid("solidColorLayer"), content);
    style.putDouble(sid("strokeStyleResolution"), 72);
    var shapeStyle = new ActionDescriptor();
    shapeStyle.putObject(sid("strokeStyle"), sid("strokeStyle"), style);
    var r = new ActionReference();
    r.putEnumerated(sid("contentLayer"), cid("Ordn"), cid("Trgt"));
    var d = new ActionDescriptor();
    d.putReference(cid("null"), r);
    d.putObject(cid("T   "), sid("shapeStyle"), shapeStyle);
    executeAction(cid("setd"), d, DialogModes.NO);
}

// Makes the document, one layer per case, saves it to outPath and closes it,
// putting the ruler units, type units and dialog setting back as they were.
// Returns a line saying what was saved.
function runCases(docName, width, height, cases, outPath) {
    var oldRuler = app.preferences.rulerUnits;
    var oldType = app.preferences.typeUnits;
    var oldDialogs = app.displayDialogs;
    var doc = null;
    try {
        app.preferences.rulerUnits = Units.PIXELS;
        app.preferences.typeUnits = TypeUnits.PIXELS;
        app.displayDialogs = DialogModes.NO;
        doc = app.documents.add(new UnitValue(width, "px"), new UnitValue(height, "px"), 72, docName,
            NewDocumentMode.RGB, DocumentFill.TRANSPARENT, 1, BitsPerChannelType.EIGHT);
        for (var c = 0; c < cases.length; c++) {
            var path = pathOf(doc, cases[c].name, cases[c].subpaths);
            fillLayerFromPath(path, cases[c].fill);
            doc.activeLayer.name = cases[c].name;
            path.remove();
            setStroke(cases[c].stroke);
        }
        var options = new PhotoshopSaveOptions();
        options.layers = true;
        options.embedColorProfile = true;
        options.alphaChannels = false;
        options.annotations = false;
        options.spotColors = false;
        doc.saveAs(new File(outPath), options, true, Extension.LOWERCASE);
        return "saved " + outPath + " with " + cases.length + " layers";
    } finally {
        if (doc !== null) {
            doc.close(SaveOptions.DONOTSAVECHANGES);
        }
        app.preferences.rulerUnits = oldRuler;
        app.preferences.typeUnits = oldType;
        app.displayDialogs = oldDialogs;
    }
}
