// Opens every document in a batch folder's written/ and saves it again into
// its photoshop/ as a PSD with its layers, so the composite it stores is
// Photoshop's own; dialogs stay off, so a document with no profile opens as
// it is. A document that will not open or save is left out and named. The
// batch folder is the script's first argument (ps_run.ps1 -Arguments). Its
// result is the count saved and each failure, or ERROR with the line and
// message when the batch itself fails.
var outcome;
var oldDialogs = app.displayDialogs;
try {
    if (arguments.length < 1) {
        throw new Error("no batch folder given");
    }
    var batch = String(arguments[0]).replace(/\\/g, "/").replace(/\/?$/, "/");
    app.displayDialogs = DialogModes.NO;
    var out = batch + "photoshop/";
    new Folder(out).create();
    var files = new Folder(batch + "written").getFiles("*.psd");
    var saved = 0;
    var failed = [];
    for (var i = 0; i < files.length; i++) {
        var doc = null;
        try {
            doc = app.open(files[i]);
            var options = new PhotoshopSaveOptions();
            options.layers = true;
            doc.saveAs(new File(out + files[i].name), options, true);
            saved++;
        } catch (e) {
            failed.push(files[i].name + ": " + e.message);
        }
        if (doc !== null) {
            doc.close(SaveOptions.DONOTSAVECHANGES);
        }
    }
    outcome = "saved " + saved + " of " + files.length + " documents to " + out + (failed.length ? "; failed: " + failed.join(" | ") : "");
} catch (e) {
    outcome = "ERROR line " + e.line + ": " + e.message;
}
app.displayDialogs = oldDialogs;
outcome;
