// The layout script: loaded while the daemon runs. Reports the layout once
// at load and on every change, never on pointer motion.

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

// The report and the two signals X01 measured come first, so nothing after
// them can keep the layout from being reported.
reportLayout();
workspace.screensChanged.connect(screensChanged);
workspace.virtualScreenGeometryChanged.connect(reportLayout);
workspace.screens.forEach(follow);
