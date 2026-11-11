// The position script: loaded only while a recording runs. Reports the
// layout once, then the pointer once at load and on every change that
// differs from the last one sent (whole logical px).

let last = "";

function reportPosition() {
    const p = workspace.cursorPos;
    const now = p.x + " " + p.y;
    if (now === last) {
        return;
    }
    last = now;
    callDBus(SERVICE, PATH, INTERFACE, "Position", TOKEN, now);
}

reportLayout();
reportPosition();
workspace.cursorPosChanged.connect(reportPosition);
