// Shared by both scripts. The daemon puts `const TOKEN = "<hex>";` before
// this: every call carries it, and the daemon accepts only the token of a
// load it made. Values travel as text, which the daemon parses once.

const SERVICE = "io.github.tobylo.AutoClicker";
const PATH = "/io/github/tobylo/AutoClicker/KWin";
const INTERFACE = "io.github.tobylo.AutoClicker.KWin";

// One output as "x y width height scale", in logical px.
function describeOutput(output) {
    const g = output.geometry;
    return [g.x, g.y, g.width, g.height, output.devicePixelRatio].join(" ");
}

// Every output, ";"-separated.
function reportLayout() {
    const outputs = workspace.screens.map(describeOutput).join(";");
    callDBus(SERVICE, PATH, INTERFACE, "Layout", TOKEN, outputs);
}
