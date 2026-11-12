//! What the watcher and the exit share: KWin, the script directory, whom
//! calls are trusted from, and KWin's owner.
//!
//! Loading the position script is: unload its plugin name, remove its files,
//! write a new file, load it and start it. An unload that fails ends the load
//! there (KWin refuses a name that is still loaded). The watcher tries a
//! failed unload again after a back-off, and every load and the exit unload
//! first, so a failure is never forgotten. A script's files stay until it is
//! unloaded, and the start sweeps any a crash left.

use std::cell::RefCell;
use std::num::NonZeroU32;

use crate::dir::ScriptDir;
use crate::display;
use crate::receiver::SharedTrust;
use crate::script::{Script, position_source};
use crate::scripting::{CallError, Scripting, bounded};
use crate::token::Token;

pub(crate) struct Session<S> {
    pub(crate) scripting: S,
    /// The checked directory, or why there is none: then nothing loads.
    pub(crate) dir: Result<ScriptDir, String>,
    pub(crate) trust: SharedTrust,
    /// `org.kde.KWin`'s owner as the watcher last saw it.
    pub(crate) owner: RefCell<Option<String>>,
    /// The process listening on the daemon's Wayland display; KWin counts
    /// only as it or its child. `None`: nothing to check against.
    pub(crate) compositor: Option<NonZeroU32>,
}

impl<S: Scripting> Session<S> {
    /// Loads and runs the position script with `token` on `owner`'s KWin.
    pub(crate) async fn load(&self, owner: &str, token: &Token) -> Result<(), CallError> {
        let script = Script::Position;
        self.unload(owner, script).await?;
        let dir = self.dir.as_ref().map_err(|why| CallError(why.clone()))?;
        let path = dir
            .write(&script.file_name(token), &position_source(token))
            .map_err(CallError)?;
        let id = bounded(self.scripting.load(owner, &path, script)).await?;
        if id < 0 {
            return Err(CallError(format!(
                "KWin refused to load {}",
                script.plugin()
            )));
        }
        bounded(self.scripting.start(owner)).await
    }

    /// Unloads `script` and removes its files.
    pub(crate) async fn unload(&self, owner: &str, script: Script) -> Result<(), CallError> {
        bounded(self.scripting.unload(owner, script)).await?;
        self.sweep(Some(script));
        Ok(())
    }

    /// Unloads every name at once, whatever was thought loaded: the position
    /// script, and a layout script an older version left. Removes the files
    /// of each one unloaded; each failure is logged.
    pub(crate) async fn unload_all(&self, owner: &str) {
        let [first, second] = Script::ALL;
        tokio::join!(
            self.unload_logged(owner, first),
            self.unload_logged(owner, second)
        );
    }

    pub(crate) async fn unload_logged(&self, owner: &str, script: Script) {
        if let Err(err) = self.unload(owner, script).await {
            log_failure("unload", script, &err);
        }
    }

    /// At exit: unloads every name from the KWin last seen, or, when the
    /// watcher never saw one (the daemon failed to start after it claimed
    /// its name, or KWin runs another display), from KWin's owner now, which
    /// may still hold what a crashed daemon left, if it runs this display.
    /// Without KWin nothing is loaded, and only files are left to remove.
    pub(crate) async fn exit(&self) {
        let seen = self.owner.borrow().clone();
        let owner = match seen {
            Some(owner) => Some(owner),
            None => self.owner_now().await,
        };
        match owner {
            Some(owner) => self.unload_all(&owner).await,
            None => self.sweep(None),
        }
    }

    /// Who owns `org.kde.KWin` now; `None` when nobody does or the bus
    /// cannot tell.
    async fn owner_now(&self) -> Option<String> {
        match bounded(self.scripting.follow_owner()).await {
            Ok((owner, _changes)) => display::checked_once(self, owner).await,
            Err(err) => {
                eprintln!("auto-clicker: cannot ask who runs KWin: {}", err.0);
                None
            }
        }
    }

    /// Removes `script`'s files; every script's when `None`.
    pub(crate) fn sweep(&self, script: Option<Script>) {
        if let Ok(dir) = &self.dir {
            dir.sweep(script);
        }
    }
}

pub(crate) fn log_failure(what: &str, script: Script, err: &CallError) {
    eprintln!(
        "auto-clicker: could not {what} the KWin script {}: {}",
        script.plugin(),
        err.0
    );
}
