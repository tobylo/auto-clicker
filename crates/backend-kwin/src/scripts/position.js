// The position script: loaded only while a recording runs. Reports the
// layout once, then the pointer once at load and on every change, in whole
// logical px. A change within one pixel is reported too, as the same point:
// it still covers the relative move behind it, which played back through
// the virtual mouse's own acceleration may cross a pixel the real one did
// not.

function reportPosition() {
    const p = workspace.cursorPos;
    callDBus(SERVICE, PATH, INTERFACE, "Position", TOKEN, p.x + " " + p.y);
}

reportLayout();
reportPosition();
workspace.cursorPosChanged.connect(reportPosition);
