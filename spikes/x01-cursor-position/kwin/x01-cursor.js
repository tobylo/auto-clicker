// X01 spike: report the pointer position and the output layout over D-Bus.
//
// KWin calls `report` on every pointer move (`Cursor::posChanged`). The
// position is whole logical pixels (`workspace.cursorPos` is a QPoint). Each
// call carries a running count of callbacks, so the receiver can tell
// callbacks from deliveries, and the script's wall-clock time in ms, as one
// string "<count> <ms>".

const SERVICE = "io.github.tobylo.AutoClickerX01";
const PATH = "/";

let callbacks = 0;

function report() {
    callbacks += 1;
    const p = workspace.cursorPos;
    callDBus(SERVICE, PATH, SERVICE, "Position", p.x, p.y, callbacks + " " + Date.now());
}

function describeOutput(output) {
    const g = output.geometry;
    return [output.name, g.x, g.y, g.width, g.height, output.devicePixelRatio].join(" ");
}

function reportLayout() {
    const g = workspace.virtualScreenGeometry;
    const outputs = workspace.screens.map(describeOutput).join(";");
    const box = [g.x, g.y, g.width, g.height].join(" ");
    callDBus(SERVICE, PATH, SERVICE, "Layout", box, outputs);
}

workspace.cursorPosChanged.connect(report);
workspace.screensChanged.connect(reportLayout);
workspace.virtualScreenGeometryChanged.connect(reportLayout);
reportLayout();
