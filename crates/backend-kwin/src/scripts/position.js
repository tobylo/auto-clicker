// The position script: loaded only while a recording runs. Reports the
// layout at load and on every change of the screens, and the pointer at load
// and on every change, in whole logical px. The daemon judges each position
// by the last layout reported before it: the reports reach it in the order
// they are sent, so a position on new screens comes after their layout
// (W01 brief, W4). A change within one pixel is reported too, as the same
// point: it still covers the relative move behind it, which played back
// through the virtual mouse's own acceleration may cross a pixel the real
// one did not.

const followed = [];

// Each output's own changes too: two screens of one size swapped keep the
// layout's size and the list of screens. An output without the signal is
// skipped rather than stopping the script.
function follow(output) {
    if (followed.indexOf(output) >= 0 || !output.geometryChanged) {
        return;
    }
    followed.push(output);
    output.geometryChanged.connect(reportLayout);
}

function screensChanged() {
    reportLayout();
    workspace.screens.forEach(follow);
}

function reportPosition() {
    // KWin rebuilds its list of outputs one at a time, so an output's
    // signal can report only some of them, and it moves the pointer before
    // it says the screens changed (workspace.cpp, updateOutputs). The
    // layout as it is now goes out first whenever it differs from the last
    // one reported.
    if (currentLayout() !== reportedLayout) {
        reportLayout();
    }
    const p = workspace.cursorPos;
    callDBus(SERVICE, PATH, INTERFACE, "Position", TOKEN, p.x + " " + p.y);
}

// The layout first, and its signals before the pointer's, so nothing after
// them can keep a change of the screens from going out first.
reportLayout();
workspace.screensChanged.connect(screensChanged);
workspace.virtualScreenGeometryChanged.connect(reportLayout);
workspace.screens.forEach(follow);
reportPosition();
workspace.cursorPosChanged.connect(reportPosition);
