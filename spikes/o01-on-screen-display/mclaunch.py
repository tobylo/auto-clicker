#!/usr/bin/env python3
"""Starts an installed Minecraft Java version without the launcher, in
offline single-player mode (from MC_DIR, ~/.minecraft by default), for the O01a sittings (on 2026-11-22 the
official launcher crashed at start after an update). Reads the version's
JSON, its libraries and its bundled Java from ~/.minecraft; downloads
nothing. Extra environment (SDL_VIDEO_DRIVER, MangoHud's) passes through.

    mclaunch.py VERSION [USERNAME]
"""

import json
import os
import re
import sys

MC = os.environ.get("MC_DIR") or os.path.expanduser("~/.minecraft")


def allowed(rules):
    """The version file's rules for Linux, without launcher features."""
    verdict = not rules
    for rule in rules or []:
        osname = rule.get("os", {}).get("name")
        if rule.get("features"):
            continue
        if osname in (None, "linux"):
            verdict = rule["action"] == "allow"
    return verdict


def classpath(version):
    jars = []
    for library in version["libraries"]:
        artifact = library.get("downloads", {}).get("artifact")
        if artifact and allowed(library.get("rules")):
            path = os.path.join(MC, "libraries", artifact["path"])
            if os.path.exists(path):
                jars.append(path)
    jars.append(os.path.join(MC, "versions", version["id"], version["id"] + ".jar"))
    return ":".join(jars)


def arguments(entries, values):
    out = []
    for entry in entries:
        if isinstance(entry, dict):
            if not allowed(entry.get("rules")):
                continue
            entry = entry["value"]
        for word in entry if isinstance(entry, list) else [entry]:
            out.append(re.sub(r"\$\{(\w+)\}", lambda m: values.get(m.group(1), ""), word))
    return out


def main():
    name = sys.argv[1]
    version = json.load(open(os.path.join(MC, "versions", name, name + ".json")))
    component = version["javaVersion"]["component"]
    java = os.path.join(MC, "runtime", component, "linux", component, "bin", "java")
    natives = os.path.join("/tmp/o01a", "natives")
    os.makedirs(natives, exist_ok=True)
    values = {
        "auth_player_name": sys.argv[2] if len(sys.argv) > 2 else "o01a",
        "version_name": name, "game_directory": MC, "assets_root": os.path.join(MC, "assets"),
        "assets_index_name": version["assets"], "auth_uuid": "00000000000000000000000000000000",
        "auth_access_token": "0", "clientid": "", "auth_xuid": "", "version_type": "release",
        "natives_directory": natives, "launcher_name": "o01a", "launcher_version": "0",
        "classpath": classpath(version),
    }
    args = arguments(version["arguments"]["jvm"], values) + [version["mainClass"]]
    args += arguments(version["arguments"]["game"], values)
    os.chdir(MC)
    os.execv(java, [java, *args])


if __name__ == "__main__":
    main()
