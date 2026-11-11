// The layout script: loaded while the daemon runs. Reports the layout once
// at load and on every change, never on pointer motion.

const followed = [];

// Each output's own changes too: two screens of one size swapped keep the
// layout's size and the list of screens.
function follow(output) {
    if (followed.indexOf(output) >= 0) {
        return;
    }
    followed.push(output);
    output.geometryChanged.connect(reportLayout);
}

function screensChanged() {
    workspace.screens.forEach(follow);
    reportLayout();
}

workspace.screens.forEach(follow);
workspace.screensChanged.connect(screensChanged);
workspace.virtualScreenGeometryChanged.connect(reportLayout);
reportLayout();
