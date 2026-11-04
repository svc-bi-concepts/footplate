//! The first run of a yard on this machine, as the commands of yardr that
//! make it: what the window's screens (window/firstrun.rs) ask is put to
//! the yardr on this machine (fetch.rs), and what yardr answers is read as
//! JSON. docs/setup.md of the yardr repository, "For a program that drives
//! the first run", is the list this follows.
//!
//! Every program here is started by its whole path or its plain name with
//! its arguments, never through a shell, with nothing on its input but
//! what a login is answered with. What a child finds on its PATH is data:
//! the PATH footplate was started with, and after it the directories
//! `yardr doctor --json` names, where the person's login shell finds a
//! program (`login_shell_path`) and where one is fetched to (`fetch`). The
//! login shells in all of it are doctor's own, and the two questions
//! `fetch::Machine` asks: where yardr is, and the PATH a terminal starts
//! with. The one file written is a shell's startup file, one line of it, and
//! only on the person's yes ([`Reach::add`]), so that a terminal finds the
//! programs that are fetched to `~/.local/bin`.

use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::fetch::{Machine, Wanted, started};

/// How long a command of yardr may take. Doctor asks the login shell, and
/// `up` waits for three servers; none of them is near this.
const LONGEST: Duration = Duration::from_secs(5 * 60);

/// The most that is read of what a command prints.
const MOST: u64 = 4 * 1024 * 1024;

/// The yardr that is run: the machine's, by its whole path, on the yard
/// whose home this is.
#[derive(Debug, Clone, PartialEq)]
pub struct Yardr {
    pub program: PathBuf,
    /// The yard's directory, which every command is told (`--home`).
    /// None is this run's yard: no flag, so the command resolves it as
    /// footplate did.
    pub home: Option<PathBuf>,
    /// The PATH of what is started.
    pub path: OsString,
    /// Who the commands act as, once the person said their name: the
    /// yard's operator, whatever the login is called.
    pub actor: Option<String>,
}

/// What a command of yardr came to: its exit status, and the one JSON
/// value it printed.
#[derive(Debug, Clone, PartialEq)]
pub struct Ran {
    pub status: i32,
    pub json: Value,
}

impl Ran {
    /// The command's own words for why it did not do what was asked.
    fn error(&self) -> Option<String> {
        self.json
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string)
    }

    /// The answer of a command that either did what was asked or says why
    /// not.
    fn done(self, what: &str) -> Result<Value, String> {
        match (self.status, self.error()) {
            (0, None) => Ok(self.json),
            (_, Some(error)) => Err(error),
            (status, None) => Err(format!("yardr {what} ended with status {status}.")),
        }
    }
}

impl Yardr {
    /// The command line of `words` (a command and its flags) with `names`
    /// after them: what a flag could be taken for stands behind `--`, so a
    /// title that starts with a dash is a title.
    fn command(&self, words: &[&str], names: &[&str]) -> Command {
        let mut command = Command::new(&self.program);
        command.args(words).arg("--json");
        if let Some(home) = &self.home {
            command.arg("--home").arg(home);
        }
        if let Some(actor) = &self.actor {
            command.arg(format!("--actor={actor}"));
        }
        if !names.is_empty() {
            command.arg("--").args(names);
        }
        command.env("PATH", &self.path);
        command
    }

    /// Runs one command to its end, with nothing to read, and reads the
    /// JSON it printed.
    pub fn run(&self, words: &[&str], names: &[&str]) -> Result<Ran, String> {
        let what = words.first().copied().unwrap_or_default();
        let mut child = started(
            self.command(words, names)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .map_err(|err| format!("yardr could not be started ({err})."))?;
        let out = child.stdout.take();
        let reading = thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(out) = out {
                let _ = out.take(MOST).read_to_end(&mut text);
            }
            text
        });
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() > LONGEST => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!(
                        "yardr {what} did not end in {} minutes, and was stopped.",
                        LONGEST.as_secs() / 60
                    ));
                }
                Ok(None) => thread::sleep(Duration::from_millis(20)),
                Err(err) => return Err(format!("yardr {what} was lost ({err}).")),
            }
        };
        let text = reading.join().unwrap_or_default();
        let json = serde_json::from_slice(&text).map_err(|_| {
            format!(
                "yardr {what} answered what footplate cannot read (status {}).",
                status.code().unwrap_or(-1)
            )
        })?;
        Ok(Ran {
            status: status.code().unwrap_or(-1),
            json,
        })
    }

    /// What this machine has of what a yard needs. Asking creates nothing.
    pub fn doctor(&self) -> Result<Doctor, String> {
        let ran = self.run(&["doctor"], &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json)
            .map_err(|err| format!("yardr doctor answered what footplate cannot read ({err})."))
    }

    /// Doctor's answer with the PATH it leads to: where it names
    /// directories the PATH does not have yet, they are added and doctor is
    /// asked once more, which then finds what is in them. The PATH only
    /// grows: a program that is found is named by doctor no longer, and
    /// what is started after must find it still.
    pub fn needs(&mut self) -> Result<Doctor, String> {
        let doctor = self.doctor()?;
        let path = widened(&self.path, &doctor);
        if path == self.path {
            return Ok(doctor);
        }
        self.path = path;
        self.doctor()
    }

    /// Creates the yard, and starts nothing. Run again it makes nothing
    /// twice.
    pub fn init(&self, form: &Form) -> Result<(), String> {
        let mut words = vec![
            "init".to_string(),
            format!("--harness={}", form.harness),
            format!("--operator={}", form.operator),
            format!("--yardmaster={}", form.yardmaster),
            format!("--style={}", form.style),
        ];
        if !form.model.is_empty() {
            words.push(format!("--model={}", form.model));
        }
        if !form.about.is_empty() {
            words.push(format!("--about={}", form.about));
        }
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        self.run(&words, &[])?.done("init").map(|_| ())
    }

    /// Starts what the yard needs that does not run: herdr's server, the
    /// yard's, the daemon.
    pub fn up(&self) -> Result<Up, String> {
        let ran = self.run(&["up"], &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json)
            .map_err(|err| format!("yardr up answered what footplate cannot read ({err})."))
    }

    /// What of the yard runs, and what of it starts at login. Asking
    /// starts nothing. [`StatusError::NoYard`] where the home holds no
    /// yard: `status` exits 2 there, and 0 wherever one is.
    pub fn status(&self) -> Result<Status, StatusError> {
        let ran = self.run(&["status"], &[]).map_err(StatusError::Failed)?;
        if ran.status == 2 {
            return Err(StatusError::NoYard);
        }
        if let Some(error) = ran.error() {
            return Err(StatusError::Failed(error));
        }
        serde_json::from_value(ran.json).map_err(|err| {
            StatusError::Failed(format!(
                "yardr status answered what footplate cannot read ({err})."
            ))
        })
    }

    /// Stops the yard: its agents, the daemon and the server. Without
    /// `yes`, on a yard with any agent, nothing is stopped: the answer
    /// asks to confirm, with the agents it would stop. With `yes` it stops
    /// them. herdr's server runs on either way. Safe to repeat.
    pub fn down(&self, yes: bool) -> Result<Down, String> {
        let words: &[&str] = match yes {
            true => &["down", "--yes"],
            false => &["down"],
        };
        let ran = self.run(words, &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json)
            .map_err(|err| format!("yardr down answered what footplate cannot read ({err})."))
    }

    /// Starts what does not run, and installs the login services: herdr's
    /// server's and the daemon's, with the web view's and the BDP
    /// server's where their settings name an address. One that is
    /// installed already is found, not installed again.
    pub fn up_login(&self) -> Result<Up, String> {
        let ran = self.run(&["up", "--login"], &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json)
            .map_err(|err| format!("yardr up answered what footplate cannot read ({err})."))
    }

    /// How the yard's agents work with its operator: `settings get`
    /// lists every setting, of which the style and the note are read.
    /// Either is empty where the yard has none set.
    pub fn operator(&self) -> Result<OperatorSettings, String> {
        let listed = self.run(&["settings", "get"], &[])?.done("settings get")?;
        let listed = listed
            .as_array()
            .ok_or("yardr settings get answered what footplate cannot read.")?;
        let value = |key: &str| {
            listed
                .iter()
                .find(|entry| entry.get("key").and_then(Value::as_str) == Some(key))
                .and_then(|entry| entry.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        Ok(OperatorSettings {
            style: value("operator_style"),
            note: value("operator_note"),
        })
    }

    /// Sets how the yard's agents work with its operator: the style, then
    /// the line about them. An empty line unsets the note, as `settings
    /// set` has it: there is no `settings unset`.
    pub fn set_operator(&self, style: &str, note: &str) -> Result<(), String> {
        self.run(&["settings", "set"], &["operator_style", style])?
            .done("settings set")?;
        self.run(&["settings", "set"], &["operator_note", note])?
            .done("settings set")?;
        Ok(())
    }

    /// Removes the daemon's login service. The manager stops the daemon
    /// it ran; one started any other way runs on.
    pub fn daemon_uninstall(&self) -> Result<Uninstalled, String> {
        let ran = self.run(&["daemon", "uninstall"], &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json).map_err(|err| {
            format!("yardr daemon uninstall answered what footplate cannot read ({err}).")
        })
    }

    /// Installs the daemon's login service, or writes it anew: its file
    /// names the yardr that runs this.
    pub fn daemon_install(&self) -> Result<(), String> {
        self.run(&["daemon", "install"], &[])?
            .done("daemon install")
            .map(|_| ())
    }

    /// Installs the web view's login service, or writes it anew: its file
    /// names the yardr that runs this. The address is the yard's own:
    /// its setting's, where it names one.
    pub fn web_install(&self) -> Result<(), String> {
        self.run(&["web", "install"], &[])?
            .done("web install")
            .map(|_| ())
    }

    /// Installs the BDP server's login service, or writes it anew: its
    /// file names the yardr that runs this. The address is the yard's
    /// own: its setting's, where it names one.
    pub fn bdp_install(&self) -> Result<(), String> {
        self.run(&["bdp", "install"], &[])?
            .done("bdp install")
            .map(|_| ())
    }

    /// What of the BDP server runs, and what of it starts at login.
    /// `status` carries no BDP service, so its server is asked itself.
    pub fn bdp_status(&self) -> Result<BdpStatus, String> {
        let ran = self.run(&["bdp", "status"], &[])?;
        if let Some(error) = ran.error() {
            return Err(error);
        }
        serde_json::from_value(ran.json)
            .map_err(|err| format!("yardr bdp status answered what footplate cannot read ({err})."))
    }

    /// Makes `dir` a depot called `name` and sets it up from the yard's
    /// pack. What the import still asks for is answered as the first run
    /// does: a group of agents works on `harness`, and a group of people
    /// is anyone, but for the decisions, which are the `operator`'s.
    pub fn depot(
        &self,
        name: &str,
        dir: &Path,
        base: Option<&str>,
        harness: &str,
        operator: &str,
    ) -> Result<(), String> {
        let dir = dir.to_str().ok_or("The folder's path is no text.")?;
        let base = base.map(|base| format!("--base={base}"));
        let mut words = vec!["depot", "add"];
        words.extend(base.as_deref());
        self.run(&words, &[name, dir])?.done("depot add")?;

        let mut answers: Vec<String> = Vec::new();
        // The import asks for what it lacks, and for nothing twice: a few
        // rounds answer it, or it is asking for what is not known here.
        for _ in 0..4 {
            let mut words = vec!["depot", "init", "--yes"];
            words.extend(answers.iter().map(String::as_str));
            let ran = self.run(&words, &[name])?;
            let missing = match ran.json.get("missing").and_then(Value::as_array) {
                Some(missing) if ran.status == 2 => missing.clone(),
                _ => return ran.done("depot init").map(|_| ()),
            };
            let before = answers.len();
            for flag in missing.iter().filter_map(Value::as_str) {
                let answer = answer(flag, harness, operator).ok_or_else(|| {
                    format!("yardr asks for something footplate has no answer to: {flag}")
                })?;
                for word in answer {
                    if !answers.contains(&word) || word.starts_with("--") {
                        answers.push(word);
                    }
                }
            }
            if answers.len() == before {
                break;
            }
        }
        Err("yardr went on asking for the depot's groups, and the depot is not set up.".to_string())
    }

    /// Creates the first bead and hands it to the builders. Its id.
    pub fn bead(&self, depot: &str, title: &str, goal: &str) -> Result<String, String> {
        let depot = format!("--depot={depot}");
        let created = self
            .run(&["bead", "create", &depot, "-b", goal], &[title])?
            .done("bead create")?;
        let id = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or("yardr created a bead and did not say which.")?
            .to_string();
        self.run(&["bead", "advance", "--to=new"], &[&id])?
            .done("bead advance")?;
        Ok(id)
    }

    /// Has `operator` told when a bead of `depot` lands.
    pub fn subscribe(&self, operator: &str, depot: &str) -> Result<(), String> {
        let depot = format!("--where=depot={depot}");
        self.run(&["subscribe", "--done", &depot], &[operator, "yardr"])?
            .done("subscribe")
            .map(|_| ())
    }
}

/// The flag that answers one entry of the import's `missing`, which is
/// `--<flag> <name>=<what is wanted>`: the flag and the name are kept, and
/// the answer put after the `=`.
fn answer(missing: &str, harness: &str, operator: &str) -> Option<[String; 2]> {
    let (flag, rest) = missing.split_once(' ')?;
    let (name, _) = rest.split_once('=')?;
    let value = match flag {
        "--bind" => harness,
        "--members" if name == "decisions" => operator,
        "--members" => "",
        _ => return None,
    };
    Some([flag.to_string(), format!("{name}={value}")])
}

/// `yardr doctor --json`, of which the machine's programs are read.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Doctor {
    /// Whether the home has a yard.
    #[serde(default)]
    pub yard: bool,
    #[serde(default)]
    pub machine: Vec<Item>,
    /// What the yard has, where there is one and its server answers.
    #[serde(default)]
    state: Option<State>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct State {
    #[serde(default)]
    yardmaster: Option<Named>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct Named {
    #[serde(default)]
    name: String,
}

/// A program a yard needs, as doctor found it or did not.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Item {
    pub program: String,
    #[serde(default, rename = "for")]
    pub what_for: String,
    #[serde(default)]
    pub found: bool,
    /// Where it is, when it is found; of a harness that is another program
    /// answering under its name, where that program is.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// How it is installed by hand, where it is missing: a page, or a
    /// command with one.
    #[serde(default)]
    pub get: Option<String>,
    /// Where the login shell finds it, when the PATH does not.
    #[serde(default)]
    pub login_shell_path: Option<PathBuf>,
    /// On a Mac: how a program fetches it.
    #[serde(default)]
    pub fetch: Option<Wanted>,
    /// On the item that says no harness is installed: the ones that can be
    /// fetched.
    #[serde(default)]
    pub any_of: Vec<Item>,
    /// The kind, of a harness.
    #[serde(default)]
    pub harness: Option<String>,
    /// The harness's program as people write it, where doctor
    /// carries it ("Claude Code").
    #[serde(default)]
    pub label: Option<String>,
    /// The harness's maker, where doctor carries it ("Anthropic").
    #[serde(default)]
    pub provider: Option<String>,
    /// The models the kind's file suggests a person pick, where it names
    /// any.
    #[serde(default)]
    pub offers: Vec<Offer>,
    /// A program can lead this kind's login.
    #[serde(default)]
    pub harness_login: bool,
    /// Of a harness doctor found: what its version flag came to. Absent
    /// from an older doctor, which asked nothing of it.
    #[serde(default)]
    pub version: Option<Version>,
    /// Of herdr: whether its server answers.
    #[serde(default)]
    pub server: Option<Server>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Server {
    #[serde(default)]
    pub state: String,
}

/// What running a harness's version flag came to (`docs/doctor.md` of the
/// yardr repository, "A program that answers as another").
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Version {
    /// `answers`, `fails`, `hangs`, `not_harness` or `not_checked`. Setup
    /// acts on `not_harness`, `fails` and `hangs` alone: any other word,
    /// one from a newer doctor too, is read as a harness that answers.
    #[serde(default)]
    pub state: String,
    /// The first line it printed; for `not_harness`, what it printed in
    /// place of the harness's name.
    #[serde(default)]
    pub output: String,
}

/// A model a harness's file suggests a person pick: the id its --model
/// flag takes, the label a person reads, and what it is recommended for,
/// where it is recommended for anything.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Offer {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub recommended: Option<String>,
}

impl Doctor {
    fn item(&self, program: &str) -> Option<&Item> {
        self.machine.iter().find(|item| item.program == program)
    }

    /// What the yard's yardmaster is called, where doctor read it.
    pub fn yardmaster(&self) -> Option<&str> {
        let named = self.state.as_ref()?.yardmaster.as_ref()?;
        (!named.name.is_empty()).then_some(named.name.as_str())
    }

    /// The harnesses doctor speaks of: its own items, and those of the
    /// item that says none is installed. A program is one harness: doctor
    /// speaks twice of another program that answers under a harness's name
    /// (its own item, with the version, and again among the ones that can be
    /// fetched), and the first is the one kept, so that the fetch is offered
    /// once and by the row that knows what stands in the way.
    pub fn harnesses(&self) -> Vec<&Item> {
        let items = self
            .machine
            .iter()
            .flat_map(|item| std::iter::once(item).chain(&item.any_of));
        let mut harnesses: Vec<&Item> = Vec::new();
        for item in items.filter(|item| item.harness.is_some()) {
            if !harnesses.iter().any(|has| has.program == item.program) {
                harnesses.push(item);
            }
        }
        harnesses
    }

    /// The kinds that are installed, in doctor's order: found, and not
    /// another program that answers under the name. Doctor says as much by
    /// `found` already; setup reads the version as well, so that it believes
    /// doctor about what the program is and not only about whether it is
    /// there.
    pub fn installed(&self) -> Vec<&Item> {
        let mut found = self.harnesses();
        found.retain(|item| item.found && !item.answers_as_another());
        found
    }

    /// The programs of the harnesses that another program answers for while
    /// the one fetched for them stands already (`stands` says whether a file
    /// does): herdr starts the first program of the name on PATH, so
    /// fetching again would change nothing.
    pub fn shadowed(&self, stands: impl Fn(&Path) -> bool) -> Vec<String> {
        self.harnesses()
            .into_iter()
            .filter(|item| item.answers_as_another())
            .filter(|item| {
                item.fetch
                    .as_ref()
                    .is_some_and(|wanted| stands(&wanted.path))
            })
            .map(|item| item.program.clone())
            .collect()
    }

    /// Whether a yard can be created: git, herdr and one harness are
    /// installed. herdr's server may still be down, which starting the
    /// yard sees to.
    pub fn ready(&self) -> bool {
        let found = |program| self.item(program).is_some_and(|item| item.found);
        found("git") && found("herdr") && !self.installed().is_empty()
    }

    /// Whether herdr is installed and its server does not answer.
    pub fn server_waits(&self) -> bool {
        self.item("herdr").is_some_and(|herdr| {
            herdr.found
                && herdr
                    .server
                    .as_ref()
                    .is_some_and(|server| server.state != "running")
        })
    }

    /// The rows of what a yard needs: git, herdr, then the harnesses.
    /// Where none is installed and doctor names none to choose from, its
    /// one line for the harness stands for them.
    pub fn rows(&self) -> Vec<&Item> {
        let mut rows: Vec<&Item> = ["git", "herdr"]
            .iter()
            .filter_map(|program| self.item(program))
            .collect();
        let harnesses = self.harnesses();
        match harnesses.is_empty() {
            true => rows.extend(self.item("a harness")),
            false => rows.extend(harnesses),
        }
        rows
    }
}

impl Offer {
    /// Whether the offer is recommended for a yardmaster: the mark can
    /// name other roles later, so only that one counts.
    pub fn for_yardmaster(&self) -> bool {
        self.recommended.as_deref() == Some("yardmaster")
    }
}

impl Item {
    /// The offer recommended for a yardmaster, where the harness's file
    /// names one.
    pub fn recommended_offer(&self) -> Option<&Offer> {
        self.offers.iter().find(|offer| offer.for_yardmaster())
    }

    /// The page its maker has for it, where doctor's hint names one: the
    /// first `https://` address of it.
    pub fn page(&self) -> Option<&str> {
        let get = self.get.as_deref()?;
        let page = &get[get.find("https://")?..];
        let end = page
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | ',' | '"' | '\''))
            .unwrap_or(page.len());
        Some(&page[..end])
    }

    /// Whether the program on PATH under this harness's name is another
    /// program: its version flag answered without saying who the harness is.
    pub fn answers_as_another(&self) -> bool {
        self.version
            .as_ref()
            .is_some_and(|version| version.state == "not_harness")
    }

    /// The harness as people write it: doctor's label, else its kind.
    fn called(&self) -> &str {
        let named = |name: &&str| !name.is_empty();
        self.label
            .as_deref()
            .filter(named)
            .or(self.harness.as_deref().filter(named))
            .unwrap_or(&self.program)
    }

    /// What the row says in place of "not installed" of a program that
    /// answers as another, in the words yardr's onboarding has for it.
    pub fn another_state(&self) -> Option<String> {
        self.answers_as_another()
            .then(|| format!("the {} on PATH is not {}", self.program, self.called()))
    }

    /// Which program that is and what it printed, for the row of a program
    /// that answers as another.
    pub fn another_said(&self) -> Option<String> {
        let version = self
            .version
            .as_ref()
            .filter(|_| self.answers_as_another())?;
        let who = match self.path.as_deref() {
            Some(path) => path.display().to_string(),
            None => format!("The {} on PATH", self.program),
        };
        let said = match printed(&version.output).as_str() {
            "" => "nothing".to_string(),
            said => format!("\"{said}\""),
        };
        Some(format!("{who} printed {said} for its version flag."))
    }

    /// What keeps the fetched program from running, for the row of a
    /// program that answers as another while the fetched one stands
    /// ([`Doctor::shadowed`]): herdr starts the first program of the name on
    /// PATH. Where the program that answers is the fetched file itself,
    /// nothing is behind it and the order of the PATH is no help: the file is
    /// in the way.
    pub fn shadow_said(&self) -> Option<String> {
        if !self.answers_as_another() {
            return None;
        }
        let fetched = self.fetch.as_ref()?.path.as_path();
        let name = self.called();
        if self.path.as_deref() == Some(fetched) {
            return Some(format!(
                "It stands where {name} would be installed, so footplate puts nothing there. Move it out of the way, then install again."
            ));
        }
        Some(format!(
            "{name} is at {} already, but herdr starts the first {} on PATH, which is the program above. Take that one off PATH, or put {} before it.",
            fetched.display(),
            self.program,
            fetched.parent()?.display()
        ))
    }

    /// The warning for a harness that is there and whose version flag
    /// failed or did not answer. It reads as installed and setup goes on: a
    /// real one that is slow or fails once on a first start must not stop
    /// it, and a session on a dead one shows its fault when it starts.
    pub fn version_warning(&self) -> Option<String> {
        let version = self.version.as_ref().filter(|_| self.found)?;
        let what = match version.state.as_str() {
            "fails" => match printed(&version.output).as_str() {
                "" => "failed".to_string(),
                said => format!("failed: \"{said}\""),
            },
            "hangs" => "did not answer".to_string(),
            _ => return None,
        };
        Some(format!(
            "{} is there, but its version flag {what}. That keeps nothing back: a session started on it shows what is wrong.",
            self.program
        ))
    }
}

/// The most chars of what a program printed that a row says.
const PRINTED: usize = 80;

/// What a program printed, as a row may say it: anything that is not text
/// taken out, as the About card has the line a yardr answers
/// ([`fetch::version_of`]), and a long line ended in an ellipsis.
fn printed(text: &str) -> String {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let text = text.trim();
    match text.char_indices().nth(PRINTED) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// The PATH of what footplate starts: `inherited`, the one it has, and
/// after it each directory doctor names that it lacks. A window
/// opened from the desktop has neither Homebrew's nor `~/.local/bin`.
pub fn widened(inherited: &OsStr, doctor: &Doctor) -> OsString {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(inherited).collect();
    let items = doctor
        .machine
        .iter()
        .flat_map(|item| std::iter::once(item).chain(&item.any_of));
    for item in items {
        let named = [
            item.login_shell_path.as_deref(),
            item.fetch.as_ref().map(|fetch| fetch.path.as_path()),
        ];
        for dir in named.into_iter().flatten().filter_map(Path::parent) {
            if dir.is_absolute() && !dirs.iter().any(|has| has == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    // A directory with the separator in its name cannot be on a PATH.
    dirs.retain(|dir| std::env::join_paths([dir]).is_ok());
    std::env::join_paths(dirs).unwrap_or_else(|_| inherited.to_os_string())
}

/// The shells whose startup file footplate knows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shell {
    Zsh,
    Bash,
    Fish,
}

impl Shell {
    /// The shell the program `$SHELL` names is, by the program's name: one
    /// of the three, else none. A shell that is not one of them is never
    /// given a file by a guess.
    pub fn of(program: Option<&Path>) -> Option<Shell> {
        match program?.file_name()?.to_str()? {
            "zsh" => Some(Shell::Zsh),
            "bash" => Some(Shell::Bash),
            "fish" => Some(Shell::Fish),
            _ => None,
        }
    }
}

/// The line that puts `~/.local/bin` first on a PATH: fish has its own
/// syntax, and every other shell is shown the one the POSIX shells read.
pub fn path_line(shell: Option<Shell>) -> &'static str {
    match shell {
        Some(Shell::Fish) => "set -gx PATH $HOME/.local/bin $PATH",
        _ => r#"export PATH="$HOME/.local/bin:$PATH""#,
    }
}

/// The file the line goes to, which a terminal's shell reads as it opens.
/// zsh's is `.zshrc` in `ZDOTDIR` where the environment sets one, else in
/// the home. A login bash reads the first of three
/// files that is there and no other, so a `.bash_profile` made beside a
/// `.profile` would hide the `.profile`: the first that is there is the
/// file, and `.bash_profile` where none is, the one Terminal's login shells
/// read.
pub fn startup_file(shell: Shell, home: &Path, zdotdir: Option<&Path>) -> PathBuf {
    match shell {
        Shell::Zsh => zdotdir
            .filter(|dir| !dir.as_os_str().is_empty())
            .unwrap_or(home)
            .join(".zshrc"),
        Shell::Bash => [".bash_profile", ".bash_login", ".profile"]
            .map(|name| home.join(name))
            .into_iter()
            .find(|file| file.exists())
            .unwrap_or_else(|| home.join(".bash_profile")),
        Shell::Fish => home.join(".config/fish/config.fish"),
    }
}

/// Whether the startup file `text` has a PATH entry that names
/// `.local/bin`: a line that holds both. A comment names nothing; the
/// stock `.zshrc` of oh-my-zsh carries one, `# export PATH=$HOME/bin:$HOME/.local/bin:…`.
pub fn names_local_bin(text: &str) -> bool {
    text.lines().any(|line| {
        !line.trim_start().starts_with('#') && line.contains("PATH") && line.contains(".local/bin")
    })
}

/// Whether `dir` is one of the directories of `path`, a PATH as a shell
/// has it.
pub fn on_path(path: &OsStr, dir: &Path) -> bool {
    std::env::split_paths(path).any(|has| has == dir)
}

/// What setup can do so that a terminal finds the programs in
/// `~/.local/bin`, where there is anything to do ([`reach`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Reach {
    /// The programs setup knows that are in the directory, by name.
    pub programs: Vec<String>,
    /// The shell the terminal runs, by the program's name ("zsh", or
    /// "tcsh" for one that is not known); none where `$SHELL` names none.
    pub shell: Option<String>,
    /// The file the line goes to. None for a shell that is not known, to
    /// which nothing is written.
    pub file: Option<PathBuf>,
    /// The line, which a person is shown whatever the shell is.
    pub line: &'static str,
    /// The person's home, which the file is said under.
    pub home: PathBuf,
}

/// What adding the line came to, where nothing failed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Added {
    /// The line is the last of the file now.
    Written,
    /// The file names `.local/bin` on a PATH already: nothing was written.
    Named,
}

impl Reach {
    /// The programs as a sentence names them: "yardr, herdr and claude".
    pub fn named(&self) -> String {
        match self.programs.split_last() {
            None => String::new(),
            Some((last, [])) => last.clone(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        }
    }

    /// The file as the person is told of it: `~/.zshrc` for one under the
    /// home.
    pub fn shown(&self) -> Option<String> {
        let file = self.file.as_deref()?;
        Some(match file.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => file.display().to_string(),
        })
    }

    /// Adds the line to the shell's file, on the person's yes, and says in
    /// the refusal's own words why not where it cannot. The line goes at
    /// the end, on a line of its own, and only where the file names
    /// `.local/bin` on a PATH nowhere, so that no run adds a second. A file
    /// that is a link is written through, and stays one.
    pub fn add(&self) -> Result<Added, String> {
        let (Some(file), Some(shown)) = (self.file.as_deref(), self.shown()) else {
            return Err(match self.shell.as_deref() {
                Some(shell) => format!(
                    "footplate does not know where {shell} keeps its startup file, so it wrote nothing."
                ),
                None => {
                    "footplate does not know which shell a terminal runs here, so it wrote nothing."
                        .to_string()
                }
            });
        };
        let text = match fs::read(file) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(format!(
                    "{shown} could not be read ({err}). Nothing was added."
                ));
            }
        };
        if names_local_bin(&String::from_utf8_lossy(&text)) {
            return Ok(Added::Named);
        }
        // One write, so that the line is never separated from the newline
        // that parts it from a last line without one.
        let mut add = Vec::new();
        if !text.is_empty() && !text.ends_with(b"\n") {
            add.push(b'\n');
        }
        add.extend_from_slice(self.line.as_bytes());
        add.push(b'\n');
        file.parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| OpenOptions::new().append(true).create(true).open(file))
            .and_then(|mut file| file.write_all(&add))
            .map(|()| Added::Written)
            .map_err(|err| format!("{shown} could not be written ({err}). Nothing was added."))
    }
}

/// What setup can do so that a terminal finds the programs setup knows
/// that are in `~/.local/bin`: where fetching puts them, and where a
/// terminal does not look. None where there is nothing to do: no such
/// program is there, or the shell's file or the PATH of its login shell
/// names the directory already. The file is asked first, which costs no
/// shell; a login shell started to read its PATH does not read zsh's
/// `.zshrc`, so only the file knows of a line that was added.
pub fn reach(machine: &Machine, yardr: &Path, doctor: &Doctor) -> Option<Reach> {
    let home = machine.home.as_deref()?;
    let dir = home.join(".local/bin");
    let programs = programs_in(&dir, yardr, doctor);
    if programs.is_empty() {
        return None;
    }
    let known = Shell::of(machine.shell.as_deref());
    let file = known.map(|shell| startup_file(shell, home, machine.zdotdir.as_deref()));
    let text = file.as_deref().and_then(|file| fs::read(file).ok());
    if text.is_some_and(|text| names_local_bin(&String::from_utf8_lossy(&text))) {
        return None;
    }
    if machine
        .login_path()
        .is_some_and(|path| on_path(&path, &dir))
    {
        return None;
    }
    Some(Reach {
        programs,
        shell: machine
            .shell
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned()),
        file,
        line: path_line(known),
        home: home.to_path_buf(),
    })
}

/// The programs in `dir`: the yardr that is run, and each program doctor
/// found there, in doctor's order.
fn programs_in(dir: &Path, yardr: &Path, doctor: &Doctor) -> Vec<String> {
    let mut programs: Vec<String> = Vec::new();
    if yardr.parent() == Some(dir) {
        programs.extend(
            yardr
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
        );
    }
    for item in doctor
        .machine
        .iter()
        .flat_map(|item| std::iter::once(item).chain(&item.any_of))
    {
        let there = item.found && item.path.as_deref().and_then(Path::parent) == Some(dir);
        if there && !programs.contains(&item.program) {
            programs.push(item.program.clone());
        }
    }
    programs
}

/// How agents work with the operator: `operator_style`, with the words
/// the onboarding has for each.
pub const STYLES: [(&str, &str, &str); 4] = [
    (
        "guide",
        "Guide me",
        "Agents explain each step in plain words before taking it, show no commands, and ask before acting for you.",
    ),
    (
        "show",
        "Show me",
        "Agents explain briefly, show the command they run, and ask when there is a decision to make.",
    ),
    (
        "work",
        "Work with me",
        "Agents are terse, assume you read code and git, and decide small things themselves.",
    ),
    (
        "report",
        "Just report",
        "Agents decide what is theirs to decide and come to you only for what is yours.",
    ),
];

/// The name of the style at `at` on the four-step scale: its number and
/// its label, from "1 · Guide me" to "4 · Just report". The setup and
/// the settings show the same scale.
pub fn style_name(at: usize) -> String {
    format!("{} · {}", at + 1, STYLES[at].1)
}

/// The place on the scale of the style called `key`, if it names one.
pub fn style_at(key: &str) -> Option<usize> {
    STYLES.iter().position(|style| style.0 == key)
}

/// How the yard's agents work with its operator, and the line they know
/// about them: `operator_style` and `operator_note`. Either is empty
/// where the yard has none set, as `settings get` lists it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OperatorSettings {
    pub style: String,
    pub note: String,
}

/// What the person said of the yard to create.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Form {
    pub operator: String,
    pub harness: String,
    pub yardmaster: String,
    pub model: String,
    pub style: String,
    pub about: String,
}

/// `yardr up --json`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Up {
    /// Whether herdr's server, the yard's and the daemon run.
    #[serde(default)]
    pub up: bool,
    #[serde(default)]
    pub parts: Vec<Part>,
    /// With `--login`: the login services, what was installed, found or
    /// left. Empty otherwise.
    #[serde(default)]
    pub login: Vec<LoginUnit>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Part {
    pub part: String,
    #[serde(default)]
    pub running: bool,
    /// What kept it from starting, and the one thing to do about it.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "do")]
    pub to_do: Option<String>,
}

impl Up {
    /// What could not be started, each with the one thing to do, in
    /// yardr's words.
    pub fn troubles(&self) -> Vec<String> {
        self.parts
            .iter()
            .filter_map(|part| {
                let error = part.error.as_deref()?;
                Some(match part.to_do.as_deref() {
                    Some(to_do) => format!("{}: {error} {to_do}", part.part),
                    None => format!("{}: {error}", part.part),
                })
            })
            .collect()
    }
}

/// `yardr status --json`, of which the switch reads what runs and what
/// starts at login.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Status {
    #[serde(default)]
    server: Runs,
    #[serde(default)]
    daemon: DaemonRuns,
    /// The login service for herdr's server: the user's, shared by every
    /// yard on the machine.
    #[serde(default, rename = "herdr_login_service")]
    herdr_login: Service,
    #[serde(default)]
    web: WebRuns,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct Runs {
    #[serde(default)]
    running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct DaemonRuns {
    #[serde(default)]
    running: bool,
    #[serde(default)]
    service: Service,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
struct WebRuns {
    #[serde(default)]
    service: Service,
}

/// A login service as yardr reports one: the daemon's, the web view's,
/// the BDP server's, or herdr's server's.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Service {
    /// The system has a service manager yardr writes to.
    #[serde(default)]
    pub supported: bool,
    /// Its file is written.
    #[serde(default)]
    pub installed: bool,
    /// The manager has it now.
    #[serde(default)]
    pub loaded: bool,
    /// Of herdr's: service files for its server that are not yardr's.
    #[serde(default)]
    pub foreign: Vec<String>,
}

impl Status {
    /// Whether the yard's server answers.
    pub fn server_running(&self) -> bool {
        self.server.running
    }

    /// Whether the daemon holds the yard's lock.
    pub fn daemon_running(&self) -> bool {
        self.daemon.running
    }

    /// The daemon's login service: whether the yard starts at login.
    pub fn daemon_service(&self) -> &Service {
        &self.daemon.service
    }

    /// The web view's login service.
    pub fn web_service(&self) -> &Service {
        &self.web.service
    }

    /// The login service for herdr's server.
    pub fn herdr_service(&self) -> &Service {
        &self.herdr_login
    }

    /// Whether the yard starts at login: the daemon's service is
    /// installed, and herdr's server starts with it, by yardr's service
    /// or one that is not yardr's. Without herdr's server the daemon
    /// starts to a yard whose agents fail.
    pub fn starts_at_login(&self) -> bool {
        self.daemon_service().installed
            && (self.herdr_service().installed || !self.herdr_service().foreign.is_empty())
    }
}

/// `yardr bdp status --json`: whether the BDP server runs, and its login
/// service.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct BdpStatus {
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub service: Service,
}

/// What `status` came to instead of the yard: the home holds no yard, or
/// yardr failed.
#[derive(Debug, Clone, PartialEq)]
pub enum StatusError {
    /// The home holds no yard: `status` exits 2 there.
    NoYard,
    Failed(String),
}

/// `yardr down --json`: without `--yes` what it would stop, with it what
/// it stopped or could not.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Down {
    /// Whether the yard is stopped now: no daemon, no server, and no
    /// agent of it left that down could not stop.
    #[serde(default)]
    pub down: bool,
    /// Nothing was stopped: the agents below have to be agreed to first,
    /// by the same command with `--yes`.
    #[serde(default)]
    pub confirm: bool,
    /// The agents the yard started: those it would stop, else those it
    /// stopped or could not.
    #[serde(default)]
    pub agents: Vec<Agent>,
    /// Those of them in the middle of something.
    #[serde(default)]
    pub working: usize,
    /// Those that are not.
    #[serde(default)]
    pub idle: usize,
    #[serde(default)]
    pub parts: Vec<Part>,
}

/// An agent the yard started and down stops: a bead's session or a crew
/// member.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Agent {
    /// bead or crew.
    #[serde(default)]
    pub kind: String,
    /// The bead, or the crew member.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stage: String,
    /// What it is doing: working (in the middle of a turn), idle,
    /// blocked (it waits at a question in its pane), unknown (it could
    /// not be asked) or gone.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub stopped: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "do")]
    pub to_do: Option<String>,
}

impl Down {
    /// The agents to name before stopping: those in the middle of
    /// something, in the order yardr lists them. The idle ones are a
    /// count ([`Down::idle`]), and one whose pane is gone is stopped
    /// already.
    pub fn named(&self) -> Vec<&Agent> {
        self.agents.iter().filter(|agent| agent.busy()).collect()
    }

    /// What could not be stopped, each with the one thing to do, in
    /// yardr's words.
    pub fn troubles(&self) -> Vec<String> {
        let agents = self.agents.iter().filter_map(|agent| {
            let error = agent.error.as_deref()?;
            Some(match agent.to_do.as_deref() {
                Some(to_do) => format!("{}: {error} {to_do}", agent.said()),
                None => format!("{}: {error}", agent.said()),
            })
        });
        let parts = self.parts.iter().filter_map(|part| {
            let error = part.error.as_deref()?;
            Some(match part.to_do.as_deref() {
                Some(to_do) => format!("{}: {error} {to_do}", part.part),
                None => format!("{}: {error}", part.part),
            })
        });
        agents.chain(parts).collect()
    }
}

impl Agent {
    /// Whether stopping it now may cost something: it is in a turn, at
    /// a question, or could not be asked.
    pub fn busy(&self) -> bool {
        matches!(self.state.as_str(), "working" | "blocked" | "unknown")
    }

    /// The agent as the warning names it: a crew member by its name, a
    /// bead by its own with its stage, and what it waits at where that
    /// is not a turn.
    pub fn said(&self) -> String {
        let mut said = match self.kind.as_str() {
            "crew" => format!("crew member {}", self.name),
            _ => match self.stage.is_empty() {
                true => format!("bead {}", self.name),
                false => format!("bead {} (stage {})", self.name, self.stage),
            },
        };
        match self.state.as_str() {
            "blocked" => said.push_str(", waits at a question in its pane"),
            "unknown" => said.push_str(", could not be asked"),
            _ => {}
        }
        said
    }
}

/// One login service in the answer of `up --login`: what was installed,
/// found or left.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct LoginUnit {
    /// herdr, daemon, web, bdp.
    #[serde(default)]
    pub part: String,
    /// What this run did: installed, found or left. Empty with an error.
    #[serde(default)]
    pub result: String,
    #[serde(default)]
    pub file: String,
    /// Whether the manager has it now.
    #[serde(default)]
    pub loaded: bool,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default, rename = "do")]
    pub to_do: Option<String>,
}

impl LoginUnit {
    /// The unit in a line, in yardr's words.
    pub(crate) fn said(&self) -> String {
        let part = match self.part.as_str() {
            "herdr" => "herdr's server",
            "daemon" => "daemon",
            "web" => "web view",
            "bdp" => "BDP server",
            part => part,
        };
        if let Some(error) = self.error.as_deref() {
            return match self.to_do.as_deref() {
                Some(to_do) => format!("{part}: NOT {error} {to_do}"),
                None => format!("{part}: NOT {error}"),
            };
        }
        let mut said = match self.result.as_str() {
            "installed" => format!("{part}: installed"),
            "found" => format!("{part}: installed already"),
            "left" => format!("{part}: left as it is"),
            result => format!("{part}: {result}"),
        };
        if !self.file.is_empty() {
            said.push_str(&format!(": {}", self.file));
        }
        if !self.detail.is_empty() {
            said.push_str(&format!(": {}", self.detail));
        }
        said
    }
}

/// `yardr daemon uninstall --json`: what went.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Uninstalled {
    /// A service was installed, and is removed.
    #[serde(default)]
    pub removed: bool,
    /// The manager had it, and has stopped what it ran.
    #[serde(default)]
    pub unloaded: bool,
}

/// The base branch to offer for the folder `dir`: the branch it has
/// checked out, where it is a git checkout. None is a folder that is none,
/// which becomes a depot of the plain kind. git is asked two things, both
/// of which only read.
pub fn branch_of(dir: &Path, path: &OsStr) -> Option<String> {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            // Nothing of the folder's own configuration is to start a
            // program for a question about its branch.
            .args(["-c", "core.fsmonitor=false", "-C"])
            .arg(dir)
            .args(args)
            .env("PATH", path)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    if git(&["rev-parse", "--is-inside-work-tree"])? != "true" {
        return None;
    }
    git(&["branch", "--show-current"])
}

/// The name a folder's depot gets to begin with: the folder's, in small
/// letters, with what is no letter, digit, `-` or `_` as a `-`.
pub fn depot_name(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name: String = name
        .chars()
        .map(|c| match c.is_alphanumeric() || matches!(c, '-' | '_') {
            true => c,
            false => '-',
        })
        .collect();
    name.trim_matches('-').to_string()
}

/// Asks Apple's own installer for the command line tools, git among them:
/// it shows its dialog and footplate waits for nothing.
pub fn apple_tools() -> Result<(), String> {
    Command::new("/usr/bin/xcode-select")
        .arg("--install")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
        .map_err(|err| format!("Apple's installer could not be asked ({err})."))
}

/// What a login needs next of the person, or how it ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Need {
    /// One of these ways to log in.
    Choose { text: String, options: Vec<String> },
    /// The page to sign in on: always an `https://` address.
    Page(String),
    /// What the page gave, to type in. It is kept nowhere.
    Input { text: String },
    /// Whether to make the one real call that proves the login.
    Confirm { text: String },
    /// The login is made; whether the harness was seen to answer.
    Done { proved: bool, said: String },
    /// The login did not come about, in yardr's words.
    Failed(String),
}

impl Need {
    /// One line of `yardr harness login --json`. A line that is none of
    /// what is known asks nothing.
    fn read(line: &str) -> Option<Need> {
        let json: Value = serde_json::from_str(line).ok()?;
        let text = |key: &str| {
            json.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if json.get("error").is_some() {
            return Some(Need::Failed(text("error")));
        }
        if json.get("done").is_some() {
            let tried = json.get("try");
            let said = tried
                .and_then(|tried| tried.get("output"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Some(Need::Done {
                proved: json.get("proved").and_then(Value::as_bool) == Some(true),
                said: said.to_string(),
            });
        }
        match json.get("need").and_then(Value::as_str)? {
            "choose" => Some(Need::Choose {
                text: text("text"),
                options: json
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            // Nothing but a page on the web is ever opened.
            "open_url" => is_page(&text("url")).then(|| Need::Page(text("url"))),
            "input" => Some(Need::Input { text: text("text") }),
            "confirm" => Some(Need::Confirm { text: text("text") }),
            _ => None,
        }
    }
}

/// Whether `url` is a page on the web, to hand to the system's opener: an
/// `https://` address with no space and no control character.
pub fn is_page(url: &str) -> bool {
    url.strip_prefix("https://").is_some_and(|rest| {
        !rest.is_empty() && !rest.chars().any(|c| c.is_whitespace() || c.is_control())
    })
}

/// A harness's login, led by `yardr harness login <kind> --json`: what it
/// needs comes out of [`Login::needs`] as yardr prints it, and what the
/// person answers goes to its input, a line each. yardr holds the
/// harness's own login command; footplate neither sees nor keeps what the
/// login makes. Dropped, the login is over: its input closes, and yardr
/// stops what it started.
pub struct Login {
    child: Option<Child>,
    input: Option<ChildStdin>,
    pub needs: async_channel::Receiver<Need>,
}

impl Login {
    pub fn start(yardr: &Yardr, kind: &str) -> Result<Login, String> {
        let mut child = started(
            yardr
                .command(&["harness", "login"], &[kind])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null()),
        )
        .map_err(|err| format!("yardr could not be started ({err})."))?;
        let (tell, needs) = async_channel::unbounded();
        let out = child.stdout.take();
        thread::spawn(move || {
            let mut ended = false;
            let lines = out.into_iter().flat_map(|out| BufReader::new(out).lines());
            for line in lines.map_while(Result::ok) {
                let Some(need) = Need::read(&line) else {
                    continue;
                };
                ended = matches!(need, Need::Done { .. } | Need::Failed(_));
                if tell.send_blocking(need).is_err() || ended {
                    break;
                }
            }
            if !ended {
                let _ = tell.send_blocking(Need::Failed(
                    "The login ended without saying how.".to_string(),
                ));
            }
        });
        Ok(Login {
            input: child.stdin.take(),
            child: Some(child),
            needs,
        })
    }

    fn say(&mut self, answer: Value) -> Result<(), String> {
        let gone = || "The login is over: it takes no answer any more.".to_string();
        let input = self.input.as_mut().ok_or_else(gone)?;
        writeln!(input, "{answer}")
            .and_then(|_| input.flush())
            .map_err(|_| gone())
    }

    /// The way to log in that was chosen.
    pub fn choose(&mut self, option: &str) -> Result<(), String> {
        self.say(json!({ "choose": option }))
    }

    /// What the page gave. It goes to yardr's input and nowhere else.
    pub fn input(&mut self, typed: String) -> Result<(), String> {
        self.say(json!({ "input": typed }))
    }

    /// Whether the one real call is made.
    pub fn confirm(&mut self, yes: bool) -> Result<(), String> {
        self.say(json!({ "confirm": yes }))
    }
}

impl Drop for Login {
    fn drop(&mut self) {
        // Its input closing is how yardr is told to stop the login.
        self.input = None;
        let Some(mut child) = self.child.take() else {
            return;
        };
        thread::spawn(move || {
            let started = Instant::now();
            while let Ok(None) = child.try_wait() {
                if started.elapsed() > Duration::from_secs(10) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        });
    }
}

/// A stand-in for yardr, for the tests of this module and of the window's
/// first run: a script that answers each command with the JSON a file
/// gives for it, and writes down what it was called with. A shell script,
/// so a unix one's.
#[cfg(all(test, unix))]
pub mod standin {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    const SCRIPT: &str = r#"#!/bin/sh
# A stand-in for yardr in footplate's tests. The answer to a command is the
# file answers/<command>.json beside this script, or answers/<command>.<n>.json
# for its n-th call where there is one; its exit status is in a file of the
# same name ending in .status, else 0.
D=$(dirname "$0")
key=$1
case "$2" in ''|-*) ;; *) key="$1-$2" ;; esac
{
    printf 'PATH=%s\n' "$PATH"
    printf 'ARGS'
    for a in "$@"; do printf ' [%s]' "$a"; done
    if [ -e "$D/old-binary" ]; then
        old=$(cat "$D/old-binary")
        if [ -e "$old" ]; then printf ' [old-present]'; else printf ' [old-gone]'; fi
    fi
    printf '\n'
} >> "$D/calls"
if [ "$key" = harness-login ]; then
    echo '{"need":"choose","text":"How do you log in to Claude Code?","options":["Claude subscription","Anthropic Console (API usage billing)"]}'
    IFS= read -r line || { echo '{"error":"stdin closed before a method was chosen"}'; exit 1; }
    printf '%s\n' "$line" >> "$D/answered"
    echo 'not json, and passed by'
    echo '{"need":"open_url","url":"https://claude.com/cai/oauth/authorize?code=true"}'
    echo '{"need":"input","text":"Paste code here if prompted >","secret":true}'
    IFS= read -r line || { echo '{"error":"stdin closed before the login command ended"}'; exit 1; }
    printf '%s\n' "$line" >> "$D/answered"
    case "$line" in *wrong*) echo '{"error":"claude auth login failed: Login failed: Request failed with status code 400"}'; exit 1 ;; esac
    echo '{"need":"confirm","text":"Check that Claude Code answers? That makes one real call to it, which costs a model call on your account with it."}'
    IFS= read -r line || { echo '{"done":true,"kind":"claude","proved":false,"try":{"state":"not_tried"}}'; exit 0; }
    printf '%s\n' "$line" >> "$D/answered"
    case "$line" in
    *true*) echo '{"done":true,"kind":"claude","proved":true,"try":{"state":"ok","output":"ok"}}' ;;
    *) echo '{"done":true,"kind":"claude","proved":false,"try":{"state":"not_tried"}}' ;;
    esac
    exit 0
fi
n=$(cat "$D/count.$key" 2>/dev/null || echo 0)
n=$((n + 1))
echo "$n" > "$D/count.$key"
f="$D/answers/$key.$n"
[ -e "$f.json" ] || f="$D/answers/$key"
if [ ! -e "$f.json" ]; then
    echo '{"error":"the stand-in has no answer to '"$key"'"}'
    exit 1
fi
cat "$f.json"
[ -e "$f.status" ] && exit "$(cat "$f.status")"
exit 0
"#;

    /// The stand-in, in `dir`: its path.
    pub fn make(dir: &Path) -> PathBuf {
        let program = dir.join("yardr");
        fs::create_dir_all(dir.join("answers")).unwrap();
        fs::write(&program, SCRIPT).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        program
    }

    /// What the stand-in beside `program` answers `key` with from here on
    /// (`doctor`, `depot-init`, or `depot-init.1` for its first call), and
    /// its exit status.
    pub fn answers(program: &Path, key: &str, status: i32, json: &serde_json::Value) {
        let answers = program.parent().unwrap().join("answers");
        fs::write(answers.join(format!("{key}.json")), json.to_string()).unwrap();
        fs::write(answers.join(format!("{key}.status")), status.to_string()).unwrap();
    }

    /// The commands the stand-in was called with, in order: the arguments
    /// of each, every one in brackets.
    pub fn calls(program: &Path) -> Vec<String> {
        let calls = fs::read_to_string(program.parent().unwrap().join("calls"));
        calls
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("ARGS "))
            .map(str::to_string)
            .collect()
    }

    /// The PATH of each call, in order.
    pub fn paths(program: &Path) -> Vec<String> {
        let calls = fs::read_to_string(program.parent().unwrap().join("calls"));
        calls
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.strip_prefix("PATH="))
            .map(str::to_string)
            .collect()
    }

    /// What a login was answered with, a line each, as it reached the
    /// stand-in's input.
    pub fn answered(program: &Path) -> Vec<String> {
        let answered = fs::read_to_string(program.parent().unwrap().join("answered"));
        answered
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// Doctor's answer on a Mac with git and nothing else: herdr and two
    /// harnesses can be fetched to `home`, one harness is by hand.
    pub fn doctor_bare(home: &Path) -> serde_json::Value {
        let bin = home.join(".local/bin");
        serde_json::json!({
            "home": home.join(".yardr"), "yard": false, "tried": false,
            "machine": [
                {"program": "herdr", "for": "it starts every agent in a pane of its own", "found": false,
                 "get": "https://herdr.dev",
                 "fetch": {"version": "0.9.4", "url": "https://github.com/herdrdev/herdr/releases/download/v0.9.4/herdr-macos-aarch64",
                           "sha256": "00", "size": 1, "path": bin.join("herdr"),
                           "verified": {"from": "https://herdr.dev/docs/install/", "on": "2026-10-13"}}},
                {"program": "git", "for": "a depot is a git repository", "found": true, "path": "/usr/bin/git"},
                {"program": "claude", "for": "a harness a yard can be created on", "found": false,
                 "get": "npm install -g @anthropic-ai/claude-code (https://claude.com/claude-code)",
                 "harness": "claude", "harness_login": true, "optional": true,
                 "login_shell_path": "/opt/stand-in/bin/claude"},
                {"program": "opencode", "for": "a harness a yard can be created on", "found": false,
                 "get": "https://opencode.ai", "harness": "opencode", "optional": true}
            ],
            "faults": ["herdr: not on PATH"]
        })
    }

    /// Doctor's answer on a Mac with no yard, git and herdr installed, and a
    /// program named claude on PATH that answers its version flag as
    /// another, in the shape yardr gives it: the item itself, not found, with
    /// the program that answered, what it printed and the way to fetch the
    /// real one; and again among the harnesses that can be fetched, which is
    /// where a machine with no harness has them.
    pub fn doctor_other(home: &Path) -> serde_json::Value {
        let bin = home.join(".local/bin");
        let get = "npm install -g @anthropic-ai/claude-code (https://claude.com/claude-code)";
        let claude = serde_json::json!({
            "program": "claude", "for": "a harness a yard can be created on", "found": false,
            "get": get, "harness": "claude", "label": "Claude Code", "provider": "Anthropic",
            "harness_login": true,
            "fetch": {"version": "2.1.296", "url": "https://downloads.claude.ai/claude/2.1.296",
                      "sha256": "00", "size": 1, "path": bin.join("claude"),
                      "run": ["install", "2.1.296"]}
        });
        let mut other = claude.clone();
        other["path"] = serde_json::json!("/usr/local/bin/claude");
        other["optional"] = serde_json::json!(true);
        other["version"] = serde_json::json!({
            "state": "not_harness", "output": "hello 1.0 from some other program"});
        serde_json::json!({
            "home": home.join(".yardr"), "yard": false, "tried": false,
            "machine": [
                {"program": "herdr", "for": "it starts every agent in a pane of its own", "found": true,
                 "path": "/opt/stand-in/bin/herdr", "server": {"state": "not_running", "output": "no server"}},
                {"program": "git", "for": "a depot is a git repository", "found": true, "path": "/usr/bin/git"},
                other,
                {"program": "a harness", "for": "the agent yardr starts: a yard is created on one",
                 "found": false, "get": "install one of the agents herdr starts (claude, codex, ...)",
                 "any_of": [
                     claude,
                     {"program": "codex", "for": "a harness a yard can be created on", "found": false,
                      "get": "npm install -g @openai/codex", "harness": "codex", "label": "Codex",
                      "fetch": {"version": "0.160.1", "url": "https://github.com/openai/codex/releases/codex.tar.gz",
                                "sha256": "00", "size": 1, "path": bin.join("codex"),
                                "member": "codex-aarch64-apple-darwin"}}]}
            ],
            "faults": ["a harness: not on PATH"]
        })
    }

    /// Status's answer for a yard that runs, or one that is stopped: the
    /// server and the daemon, and the login services where `login` says
    /// the yard starts at login.
    pub fn status(up: bool, login: bool) -> serde_json::Value {
        let service = |installed: bool| {
            serde_json::json!({"supported": true, "manager": "launchd",
                "name": "dev.yardr.test", "file": "/tmp/test.plist",
                "installed": installed, "loaded": installed})
        };
        serde_json::json!({
            "home": "/tmp/test-yard", "name": "test", "operator": "Jo",
            "server": {"running": up, "cli_build": "test"},
            "daemon": {"running": up, "pid": 0, "service": service(login)},
            "herdr_login_service": service(login),
            "web": {"views": [], "service": service(false)},
        })
    }

    /// Down's answer without `--yes` on a yard with agents: nothing is
    /// stopped, and the agents it would stop ask to be agreed to. One
    /// bead's session is in the middle of a turn, another waits at a
    /// question, two are idle, and one's pane is gone.
    pub fn down_confirm() -> serde_json::Value {
        serde_json::json!({
            "home": "/tmp/test-yard", "down": false, "confirm": true,
            "agents": [
                {"kind": "bead", "name": "essays-1a2b", "stage": "new",
                 "group": "builders", "state": "working", "stopped": false},
                {"kind": "crew", "name": "yardmaster", "state": "blocked", "stopped": false},
                {"kind": "bead", "name": "essays-3c4d", "stage": "review",
                 "group": "builders", "state": "idle", "stopped": false},
                {"kind": "bead", "name": "essays-5e6f", "stage": "new",
                 "group": "builders", "state": "idle", "stopped": false},
                {"kind": "bead", "name": "essays-7g8h", "stage": "new",
                 "group": "builders", "state": "gone", "stopped": false}
            ],
            "working": 2, "idle": 2,
            "parts": [{"part": "daemon", "running": true}]
        })
    }

    /// Down's answer with `--yes`: the agents it stopped, and the yard
    /// stopped.
    pub fn down_done() -> serde_json::Value {
        serde_json::json!({
            "home": "/tmp/test-yard", "down": true, "confirm": false,
            "agents": [
                {"kind": "bead", "name": "essays-1a2b", "stage": "new",
                 "group": "builders", "state": "working", "stopped": true}
            ],
            "working": 1, "idle": 0,
            "parts": [{"part": "daemon", "running": false, "stopped": true}]
        })
    }

    /// Doctor's answer once herdr and the harnesses are there: herdr's
    /// server is down, which keeps nothing from being created.
    pub fn doctor_ready() -> serde_json::Value {
        serde_json::json!({
            "yard": false,
            "machine": [
                {"program": "herdr", "for": "it starts every agent in a pane of its own", "found": true,
                 "path": "/opt/stand-in/bin/herdr", "server": {"state": "not_running", "output": "no server"}},
                {"program": "git", "for": "a depot is a git repository", "found": true, "path": "/usr/bin/git"},
                {"program": "claude", "for": "a harness a yard can be created on", "found": true,
                 "path": "/opt/stand-in/bin/claude", "harness": "claude", "harness_login": true},
                {"program": "codex", "for": "a harness a yard can be created on", "found": true,
                 "path": "/opt/stand-in/bin/codex", "harness": "codex"}
            ],
            "faults": []
        })
    }

    /// Doctor's answer where the harnesses name their maker and
    /// program and the models they offer, one recommended for a
    /// yardmaster each.
    pub fn doctor_ready_with_offers() -> serde_json::Value {
        serde_json::json!({
            "yard": false,
            "machine": [
                {"program": "herdr", "for": "it starts every agent in a pane of its own", "found": true,
                 "path": "/opt/stand-in/bin/herdr", "server": {"state": "not_running", "output": "no server"}},
                {"program": "git", "for": "a depot is a git repository", "found": true, "path": "/usr/bin/git"},
                {"program": "claude", "for": "a harness a yard can be created on", "found": true,
                 "path": "/opt/stand-in/bin/claude", "harness": "claude", "harness_login": true,
                 "provider": "Anthropic", "label": "Claude Code",
                 "offers": [
                     {"model": "opus", "label": "Opus 5.5", "recommended": "yardmaster"},
                     {"model": "fable", "label": "Fable 5.1"},
                     {"model": "sonnet", "label": "Sonnet 5"}]},
                {"program": "codex", "for": "a harness a yard can be created on", "found": true,
                 "path": "/opt/stand-in/bin/codex", "harness": "codex",
                 "provider": "OpenAI", "label": "Codex",
                 "offers": [
                     {"model": "gpt-6-luna", "label": "GPT-6 Luna", "recommended": "yardmaster"},
                     {"model": "gpt-6-astra", "label": "GPT-6 Astra"}]}
            ],
            "faults": []
        })
    }
}

// The tests run the stand-in, which only a unix system has a shell for.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use serde_json::json;

    fn yardr(dir: &Path) -> Yardr {
        Yardr {
            program: standin::make(dir),
            home: Some(dir.join("home/.yardr")),
            path: OsString::from("/usr/bin:/bin"),
            actor: None,
        }
    }

    /// Doctor's rows come from its JSON, and the PATH of what is started
    /// after it from its data: the directories the login shell found a
    /// program in and those a program is fetched to, after the PATH
    /// footplate has. Doctor is asked again with it, and never a shell.
    #[test]
    fn the_path_of_what_is_started_is_built_from_doctor_s_data() {
        let dir = tempfile::tempdir().unwrap();
        let mut yardr = yardr(dir.path());
        let home = dir.path().join("home");
        standin::answers(&yardr.program, "doctor", 1, &standin::doctor_bare(&home));
        let doctor = yardr.needs().unwrap();
        let local = home.join(".local/bin");
        let wide = format!("/usr/bin:/bin:{}:/opt/stand-in/bin", local.display());
        assert_eq!(yardr.path, OsString::from(&wide));
        // Asked twice: as footplate was started, then with what it named.
        assert_eq!(standin::paths(&yardr.program), ["/usr/bin:/bin", &wide]);
        let call = format!(
            "[doctor] [--json] [--home] [{}]",
            dir.path().join("home/.yardr").display()
        );
        assert_eq!(standin::calls(&yardr.program), [call.clone(), call]);

        assert!(!doctor.ready());
        let rows: Vec<&str> = doctor.rows().iter().map(|r| r.program.as_str()).collect();
        assert_eq!(rows, ["git", "herdr", "claude", "opencode"]);
        let herdr = doctor.item("herdr").unwrap();
        assert_eq!(herdr.fetch.as_ref().unwrap().path, local.join("herdr"));
        assert_eq!(herdr.page(), Some("https://herdr.dev"));
        let claude = doctor.item("claude").unwrap();
        assert_eq!(claude.page(), Some("https://claude.com/claude-code"));
        assert!(claude.harness_login);

        // A PATH that has them already is left as it is, and doctor asked
        // once. Nor does it lose what it has once doctor finds the
        // programs and names their directories no longer.
        yardr.needs().unwrap();
        assert_eq!(standin::calls(&yardr.program).len(), 3);
        standin::answers(&yardr.program, "doctor", 0, &standin::doctor_ready());
        yardr.needs().unwrap();
        assert_eq!(yardr.path, OsString::from(&wide));
        assert_eq!(standin::paths(&yardr.program)[3], wide);
    }

    /// A herdr whose server is down waits, and keeps nothing from being
    /// created; a machine with no harness does.
    #[test]
    fn a_stopped_herdr_server_does_not_keep_a_yard_from_being_created() {
        let doctor: Doctor = serde_json::from_value(standin::doctor_ready()).unwrap();
        assert!(doctor.ready() && doctor.server_waits());
        assert_eq!(doctor.yardmaster(), None);
        let running: Doctor = serde_json::from_value(
            json!({"yard": true, "state": {"server": "running", "yardmaster": {"name": "ym"}}}),
        )
        .unwrap();
        assert_eq!(running.yardmaster(), Some("ym"));
        let kinds: Vec<_> = doctor
            .installed()
            .iter()
            .map(|item| item.harness.clone().unwrap())
            .collect();
        assert_eq!(kinds, ["claude", "codex"]);

        // No harness, on Linux: doctor's one line for it is the row.
        let none: Doctor = serde_json::from_value(json!({"machine": [
            {"program": "git", "found": true}, {"program": "herdr", "found": true},
            {"program": "a harness", "found": false, "get": "https://claude.com/claude-code"}
        ]}))
        .unwrap();
        assert!(!none.ready() && !none.server_waits());
        assert_eq!(none.rows().len(), 3);
        // And on a Mac it names the ones that can be fetched.
        let some: Doctor = serde_json::from_value(json!({"machine": [
            {"program": "a harness", "found": false, "any_of": [
                {"program": "claude", "harness": "claude", "found": false,
                 "fetch": {"version": "1", "url": "https://x/y", "sha256": "00", "size": 1, "path": "/home/jo/.local/bin/claude"}}]}
        ]}))
        .unwrap();
        assert_eq!(some.rows().len(), 1);
        assert_eq!(some.rows()[0].program, "claude");
        assert_eq!(
            widened(OsStr::new("/bin"), &some),
            OsString::from("/bin:/home/jo/.local/bin")
        );
    }

    /// A harness names its maker and program and the models it
    /// offers, one recommended for a yardmaster; one that names none
    /// of them has none, and only the yardmaster's mark counts.
    #[test]
    fn a_harness_names_its_label_and_the_models_it_offers() {
        let doctor: Doctor = serde_json::from_value(standin::doctor_ready_with_offers()).unwrap();
        assert!(doctor.ready());
        let claude = doctor.item("claude").unwrap();
        assert_eq!(claude.provider.as_deref(), Some("Anthropic"));
        assert_eq!(claude.label.as_deref(), Some("Claude Code"));
        assert_eq!(claude.offers.len(), 3);
        assert_eq!(
            claude.recommended_offer().unwrap().model,
            "opus".to_string()
        );
        let codex = doctor.item("codex").unwrap();
        assert_eq!(codex.provider.as_deref(), Some("OpenAI"));
        assert_eq!(codex.label.as_deref(), Some("Codex"));
        assert_eq!(
            codex.recommended_offer().unwrap().model,
            "gpt-6-luna".to_string()
        );

        // Without either, there is nothing to pick: the kind stands for
        // the name, and the harness's own choice for the model.
        let bare: Doctor = serde_json::from_value(standin::doctor_ready()).unwrap();
        let claude = bare.item("claude").unwrap();
        assert_eq!(claude.provider, None);
        assert_eq!(claude.label, None);
        assert!(claude.offers.is_empty());
        assert_eq!(claude.recommended_offer(), None);

        // A mark for another role is not the yardmaster's.
        let other: Item = serde_json::from_value(json!({
            "program": "hal", "harness": "hal",
            "offers": [{"model": "hal-1", "label": "Hal 1", "recommended": "builder"}]
        }))
        .unwrap();
        assert_eq!(other.recommended_offer(), None);
    }

    /// Setup believes doctor about a harness's version, and not only about
    /// whether it is found: another program answering under the name is not
    /// installed, and its row says which program it is and what it printed; a
    /// flag that fails or hangs warns and stops nothing; a state setup does
    /// not know, and a doctor that says none, read as they always did.
    #[test]
    fn a_harness_s_version_check_decides_whether_it_is_installed_and_what_its_row_says() {
        // git, herdr and a claude of the given kind.
        let doctor_of = |found: bool, version: serde_json::Value| -> Doctor {
            let mut claude = json!({"program": "claude", "harness": "claude", "label": "Claude Code",
                "found": found, "path": "/usr/local/bin/claude"});
            if !version.is_null() {
                claude["version"] = version;
            }
            serde_json::from_value(json!({"machine": [
                {"program": "git", "found": true}, {"program": "herdr", "found": true}, claude]}))
            .unwrap()
        };
        let warns = |what: &str| {
            format!(
                "claude is there, but its version flag {what}. That keeps nothing back: a session started on it shows what is wrong."
            )
        };
        let hello = json!({"state": "not_harness", "output": "hello 1.0"});
        // The version doctor gives, whether it is found, and what follows:
        // whether a yard can be created, and the warning on the row.
        let cases = [
            // An older doctor, a program that answers and one not asked
            // read as they always did.
            (true, json!(null), true, None),
            (
                true,
                json!({"state": "answers", "output": "2.1.310 (Claude Code)"}),
                true,
                None,
            ),
            (true, json!({"state": "not_checked"}), true, None),
            // A state of a newer doctor is no reason to stop.
            (
                true,
                json!({"state": "asleep", "output": "zzz"}),
                true,
                None,
            ),
            // A flag that fails or hangs warns, and setup goes on.
            (
                true,
                json!({"state": "fails", "output": "exit status 1"}),
                true,
                Some(warns("failed: \"exit status 1\"")),
            ),
            (true, json!({"state": "fails"}), true, Some(warns("failed"))),
            (
                true,
                json!({"state": "hangs"}),
                true,
                Some(warns("did not answer")),
            ),
            // Another program is not the harness, as doctor says by not
            // finding it, and as it is when a doctor does not.
            (false, hello.clone(), false, None),
            (true, hello.clone(), false, None),
        ];
        for (found, version, installed, warning) in cases {
            let doctor = doctor_of(found, version.clone());
            let claude = doctor.item("claude").unwrap();
            assert_eq!(
                doctor.installed().len(),
                usize::from(installed),
                "{found} {version}"
            );
            assert_eq!(doctor.ready(), installed, "{found} {version}");
            assert_eq!(claude.version_warning(), warning, "{found} {version}");
        }

        // The row of a program that answers as another names the harness it
        // is not, which program it is and what it printed.
        let item = |path: Option<&str>, output: &str| -> Item {
            let mut item = json!({"program": "claude", "harness": "claude", "label": "Claude Code",
                "found": false, "version": {"state": "not_harness", "output": output}});
            if let Some(path) = path {
                item["path"] = json!(path);
            }
            serde_json::from_value(item).unwrap()
        };
        let at = Some("/usr/local/bin/claude");
        assert!(item(at, "").answers_as_another());
        assert_eq!(
            item(at, "").another_state().as_deref(),
            Some("the claude on PATH is not Claude Code")
        );
        let said = |path, output: &str| item(path, output).another_said();
        assert_eq!(
            said(at, "hello 1.0").as_deref(),
            Some(r#"/usr/local/bin/claude printed "hello 1.0" for its version flag."#)
        );
        // Nothing printed, and no place known.
        assert_eq!(
            said(at, "").as_deref(),
            Some("/usr/local/bin/claude printed nothing for its version flag.")
        );
        assert_eq!(
            said(None, "hi").as_deref(),
            Some(r#"The claude on PATH printed "hi" for its version flag."#)
        );
        // What is no text is left out, and a line past 80 chars is cut.
        assert_eq!(
            said(at, " he\u{7}l\u{1b}lo\n").as_deref(),
            Some(r#"/usr/local/bin/claude printed "hello" for its version flag."#)
        );
        let (whole, long) = ("x".repeat(80), "x".repeat(81));
        assert_eq!(
            said(at, &whole),
            Some(format!(
                r#"/usr/local/bin/claude printed "{whole}" for its version flag."#
            ))
        );
        assert_eq!(
            said(at, &long),
            Some(format!(
                r#"/usr/local/bin/claude printed "{whole}…" for its version flag."#
            ))
        );
        // A label doctor does not carry is the kind.
        let mut bare = item(at, "x");
        bare.label = None;
        assert_eq!(
            bare.another_state().as_deref(),
            Some("the claude on PATH is not claude")
        );
        // None of this is said of a harness that answers.
        let answers = doctor_of(true, json!({"state": "answers", "output": "2.1.310"}));
        let claude = answers.item("claude").unwrap();
        assert!(!claude.answers_as_another());
        assert_eq!(
            [
                claude.another_state(),
                claude.another_said(),
                claude.shadow_said()
            ],
            [None, None, None]
        );
    }

    /// With no yard, doctor speaks of a program that answers as another
    /// twice: as itself, with its version, and again among the harnesses that
    /// can be fetched. It is one row, the first, so that the fetch is
    /// offered once; and once the fetched file stands it is offered no more,
    /// since the other program is first on PATH and herdr starts that.
    #[test]
    fn a_program_that_answers_as_another_has_one_row_and_a_fetch_that_stands_is_not_offered_again()
    {
        let home = Path::new("/Users/jo");
        let fetched = home.join(".local/bin/claude");
        let doctor: Doctor = serde_json::from_value(standin::doctor_other(home)).unwrap();
        let rows = doctor.rows();
        let programs: Vec<&str> = rows.iter().map(|row| row.program.as_str()).collect();
        assert_eq!(programs, ["git", "herdr", "claude", "codex"]);
        // The row that stands is the one that knows the version, with the
        // fetch that an install of its program finds.
        let claude = rows[2];
        assert!(claude.answers_as_another());
        assert_eq!(claude.fetch.as_ref().unwrap().path, fetched);
        assert!(doctor.installed().is_empty() && !doctor.ready());

        // Nothing stands: nothing is shadowed. The file stands: claude is,
        // and codex, which no other program answers for, is not.
        assert!(doctor.shadowed(|_| false).is_empty());
        assert_eq!(doctor.shadowed(|place| place == fetched), ["claude"]);
        assert_eq!(doctor.shadowed(|_| true), ["claude"]);
        assert!(
            doctor
                .shadowed(|place| place == home.join(".local/bin/codex"))
                .is_empty()
        );

        // What keeps it from running: the program above, first on PATH.
        assert_eq!(
            claude.shadow_said().as_deref(),
            Some(
                "Claude Code is at /Users/jo/.local/bin/claude already, but herdr starts the first claude on PATH, which is the program above. Take that one off PATH, or put /Users/jo/.local/bin before it."
            )
        );
        // Where the program that answers is the fetched file itself, nothing
        // is behind it: it is in the way.
        let mut in_place = claude.clone();
        in_place.path = Some(fetched);
        assert_eq!(
            in_place.shadow_said().as_deref(),
            Some(
                "It stands where Claude Code would be installed, so footplate puts nothing there. Move it out of the way, then install again."
            )
        );
        // Not of a program that no other answers for, nor one with no fetch.
        assert_eq!(rows[3].shadow_said(), None);
        let mut unfetched = claude.clone();
        unfetched.fetch = None;
        assert_eq!(unfetched.shadow_said(), None);
    }

    /// The commands of the first run, as yardr's documentation lists them
    /// for a program: each with `--json`, the yard's home and who acts,
    /// and what could be read as a flag behind `--`.
    #[test]
    fn the_first_run_s_commands_are_arguments_and_never_a_shell_s_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut yardr = yardr(dir.path());
        yardr.actor = Some("Jo".to_string());
        let program = yardr.program.clone();
        let home = yardr.home.as_deref().unwrap().display().to_string();
        let tail = format!("[--json] [--home] [{home}] [--actor=Jo]");

        standin::answers(&program, "init", 0, &json!({"harness": "claude"}));
        let form = Form {
            operator: "Jo".into(),
            harness: "claude".into(),
            yardmaster: "yardmaster".into(),
            model: String::new(),
            style: "guide".into(),
            about: "$(touch /tmp/x); `id`".into(),
        };
        yardr.init(&form).unwrap();

        standin::answers(
            &program,
            "up",
            1,
            &json!({"up": false, "parts": [
            {"part": "herdr", "running": false, "error": "herdr is not on PATH.", "do": "Install herdr."},
            {"part": "server", "running": true}]}),
        );
        let up = yardr.up().unwrap();
        assert!(!up.up);
        assert_eq!(
            up.troubles(),
            ["herdr: herdr is not on PATH. Install herdr."]
        );

        standin::answers(
            &program,
            "depot-add",
            0,
            &json!({"name": "essays", "kind": "git"}),
        );
        standin::answers(
            &program,
            "depot-init.1",
            2,
            &json!({
            "error": "depot init cannot ask without a terminal",
            "missing": ["--bind builders=<kind>[:<model>]", "--bind reviewers=<kind>[:<model>]",
                        "--members backlog=<users>", "--members decisions=<users>"]}),
        );
        standin::answers(
            &program,
            "depot-init",
            0,
            &json!({"result": {"created": 9}}),
        );
        let folder = dir.path().join("My Essays");
        yardr
            .depot("essays", &folder, Some("main"), "claude", "Jo")
            .unwrap();

        standin::answers(
            &program,
            "bead-create",
            0,
            &json!({"id": "essays-1a2b", "stage": "backlog"}),
        );
        standin::answers(
            &program,
            "bead-advance",
            0,
            &json!({"id": "essays-1a2b", "stage": "new"}),
        );
        assert_eq!(
            yardr
                .bead("essays", "-rf is a title", "Done when it reads well.")
                .unwrap(),
            "essays-1a2b"
        );
        standin::answers(&program, "subscribe", 0, &json!([]));
        yardr.subscribe("Jo", "essays").unwrap();

        let answers = "[--bind] [builders=claude] [--bind] [reviewers=claude] [--members] [backlog=] [--members] [decisions=Jo]";
        assert_eq!(
            standin::calls(&program),
            [
                format!(
                    "[init] [--harness=claude] [--operator=Jo] [--yardmaster=yardmaster] [--style=guide] [--about=$(touch /tmp/x); `id`] {tail}"
                ),
                format!("[up] {tail}"),
                format!(
                    "[depot] [add] [--base=main] {tail} [--] [essays] [{}]",
                    folder.display()
                ),
                format!("[depot] [init] [--yes] {tail} [--] [essays]"),
                format!("[depot] [init] [--yes] {answers} {tail} [--] [essays]"),
                format!(
                    "[bead] [create] [--depot=essays] [-b] [Done when it reads well.] {tail} [--] [-rf is a title]"
                ),
                format!("[bead] [advance] [--to=new] {tail} [--] [essays-1a2b]"),
                format!("[subscribe] [--done] [--where=depot=essays] {tail} [--] [Jo] [yardr]"),
            ]
        );
    }

    /// The switch's commands are arguments too: status, down with and
    /// without the yes-flag, up with login, and the daemon's uninstall.
    /// This run's yard takes no `--home`; another home's takes its own.
    #[test]
    fn the_switch_s_commands_take_a_home_or_none_as_the_yard_is() {
        let dir = tempfile::tempdir().unwrap();
        let program = standin::make(dir.path());
        let home = dir.path().join("second/.yardr");
        let mut yardr = Yardr {
            program: program.clone(),
            home: None,
            path: OsString::from("/usr/bin:/bin"),
            actor: None,
        };
        standin::answers(&program, "status", 0, &standin::status(false, false));
        let status = yardr.status().unwrap();
        assert!(!status.server_running() && !status.daemon_running());
        assert!(!status.daemon_service().installed);

        standin::answers(&program, "down", 0, &standin::down_done());
        let down = yardr.down(true).unwrap();
        assert!(down.down && !down.confirm);

        // Another home's yard: its socket minus `yard.sock`.
        yardr.home = Some(home.clone());
        standin::answers(&program, "up", 0, &json!({"up": true, "parts": []}));
        assert!(yardr.up().unwrap().up);
        assert_eq!(
            standin::calls(&program),
            [
                "[status] [--json]".to_string(),
                "[down] [--yes] [--json]".to_string(),
                format!("[up] [--json] [--home] [{}]", home.display()),
            ]
        );
    }

    /// Status on a home with no yard is exit 2, which is no yard and no
    /// failure; any other refusal is yardr's words.
    #[test]
    fn status_on_a_home_with_no_yard_is_no_yard() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        standin::answers(
            &yardr.program,
            "status",
            2,
            &json!({"error": "no yard in /tmp/x yet: create one with `yardr init --harness <kind>`"}),
        );
        assert_eq!(yardr.status().unwrap_err(), StatusError::NoYard);
        standin::answers(
            &yardr.program,
            "status",
            1,
            &json!({"error": "the store cannot be read"}),
        );
        assert_eq!(
            yardr.status().unwrap_err(),
            StatusError::Failed("the store cannot be read".to_string())
        );
    }

    /// Starting at login is both halves: the daemon's service, and
    /// herdr's server starting with it, by yardr's service or one that is
    /// not yardr's. Either half missing is off: checking installs what is
    /// missing.
    #[test]
    fn starting_at_login_is_the_daemon_s_service_and_herdr_s() {
        let status = |daemon: bool, herdr: bool, foreign: &[&str]| {
            serde_json::from_value::<Status>(json!({
                "daemon": {"running": false,
                           "service": {"supported": true, "installed": daemon}},
                "herdr_login_service": {"supported": true, "installed": herdr,
                                        "foreign": foreign},
            }))
            .unwrap()
        };
        assert!(status(true, true, &[]).starts_at_login());
        assert!(status(true, false, &["/tmp/herdr.plist"]).starts_at_login());
        assert!(!status(true, false, &[]).starts_at_login());
        assert!(!status(false, true, &[]).starts_at_login());
        assert!(!status(false, false, &[]).starts_at_login());
    }

    /// Down without the flag asks first: the agents in the middle of
    /// something are named, the idle ones counted, and one's gone pane is
    /// neither. What could not be stopped is yardr's words.
    #[test]
    fn down_without_yes_names_what_it_would_stop() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        standin::answers(&yardr.program, "down", 1, &standin::down_confirm());
        let down = yardr.down(false).unwrap();
        assert!(!down.down && down.confirm);
        assert_eq!(down.working, 2);
        assert_eq!(down.idle, 2);
        let named: Vec<String> = down.named().iter().map(|agent| agent.said()).collect();
        assert_eq!(
            named,
            [
                "bead essays-1a2b (stage new)",
                "crew member yardmaster, waits at a question in its pane",
            ]
        );
        assert!(down.troubles().is_empty());

        standin::answers(
            &yardr.program,
            "down",
            1,
            &json!({"down": false, "confirm": false,
                "agents": [{"kind": "crew", "name": "yardmaster", "state": "working",
                            "stopped": false, "error": "herdr does not answer"}],
                "working": 1, "idle": 0,
                "parts": [{"part": "daemon", "running": true,
                           "error": "the daemon would not stop", "do": "kill it"}]}),
        );
        let down = yardr.down(true).unwrap();
        assert!(!down.down && !down.confirm);
        assert_eq!(
            down.troubles(),
            [
                "crew member yardmaster: herdr does not answer",
                "daemon: the daemon would not stop kill it",
            ]
        );
    }

    /// Up with login says per service what it did, in yardr's words; the
    /// daemon's uninstall says what went.
    #[test]
    fn up_login_installs_and_the_daemon_s_uninstall_removes() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        standin::answers(
            &yardr.program,
            "up",
            0,
            &json!({"up": true, "parts": [],
                "login": [
                    {"part": "herdr", "result": "installed", "file": "/tmp/x", "loaded": true},
                    {"part": "daemon", "result": "found", "file": "/tmp/y", "loaded": true},
                    {"part": "web", "result": "left",
                     "detail": "the yard's web_addr setting is unset"}]}),
        );
        let up = yardr.up_login().unwrap();
        assert!(up.up);
        let said: Vec<String> = up.login.iter().map(|unit| unit.said()).collect();
        assert_eq!(
            said,
            [
                "herdr's server: installed: /tmp/x",
                "daemon: installed already: /tmp/y",
                "web view: left as it is: the yard's web_addr setting is unset",
            ]
        );

        standin::answers(
            &yardr.program,
            "daemon-uninstall",
            0,
            &json!({"kind": "daemon", "name": "x", "file": "/tmp/y",
                    "removed": true, "unloaded": true}),
        );
        let uninstalled = yardr.daemon_uninstall().unwrap();
        assert!(uninstalled.removed && uninstalled.unloaded);

        let home = dir.path().join("home/.yardr");
        assert_eq!(
            standin::calls(&yardr.program),
            [
                format!("[up] [--login] [--json] [--home] [{}]", home.display()),
                format!(
                    "[daemon] [uninstall] [--json] [--home] [{}]",
                    home.display()
                ),
            ]
        );
    }

    /// The four styles are a numbered scale: each number with its
    /// label, and each key with its place. The setup and the settings
    /// show the same row.
    #[test]
    fn the_styles_are_a_numbered_scale() {
        assert_eq!(
            (0..STYLES.len()).map(style_name).collect::<Vec<_>>(),
            [
                "1 · Guide me",
                "2 · Show me",
                "3 · Work with me",
                "4 · Just report",
            ]
        );
        assert_eq!(style_at("guide"), Some(0));
        assert_eq!(style_at("report"), Some(3));
        assert_eq!(style_at(""), None);
        assert_eq!(style_at("fifth"), None);
    }

    /// The operator's settings are read as a pair and set one by one: an
    /// empty line unsets the note, as `settings set` has it. The key and
    /// the value stand behind `--`, so a line starting with a dash is one.
    #[test]
    fn the_operator_s_settings_are_read_as_a_pair_and_set_one_by_one() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        standin::answers(
            &yardr.program,
            "settings-get",
            0,
            &json!([
                {"key": "operator_style", "value": "work", "from": "set"},
                {"key": "operator_note", "value": "", "from": ""},
            ]),
        );
        assert_eq!(
            yardr.operator().unwrap(),
            OperatorSettings {
                style: "work".into(),
                note: "".into(),
            }
        );
        // What is no list is not read as one.
        standin::answers(
            &yardr.program,
            "settings-get.2",
            0,
            &json!({"key": "operator_style", "value": "work", "from": "set"}),
        );
        assert_eq!(
            yardr.operator().unwrap_err(),
            "yardr settings get answered what footplate cannot read."
        );

        standin::answers(
            &yardr.program,
            "settings-set",
            0,
            &json!({"operator_style": "report"}),
        );
        yardr
            .set_operator("report", "-a line that starts with a dash")
            .unwrap();
        let home = dir.path().join("home/.yardr");
        assert_eq!(
            standin::calls(&yardr.program),
            [
                format!("[settings] [get] [--json] [--home] [{}]", home.display()),
                format!("[settings] [get] [--json] [--home] [{}]", home.display()),
                format!(
                    "[settings] [set] [--json] [--home] [{}] [--] [operator_style] [report]",
                    home.display()
                ),
                format!(
                    "[settings] [set] [--json] [--home] [{}] [--] [operator_note] [-a line that starts with a dash]",
                    home.display()
                ),
            ]
        );
    }

    /// What yardr refuses is said in its words, and an import that asks
    /// for what footplate has no answer to is not answered with a guess.
    #[test]
    fn a_refusal_is_yardr_s_words_and_an_unknown_question_ends_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        standin::answers(
            &yardr.program,
            "init",
            1,
            &json!({"error": "unknown harness \"hal\""}),
        );
        let form = Form {
            harness: "hal".into(),
            ..Form::default()
        };
        assert_eq!(yardr.init(&form).unwrap_err(), "unknown harness \"hal\"");

        standin::answers(&yardr.program, "depot-add", 0, &json!({"name": "essays"}));
        standin::answers(
            &yardr.program,
            "depot-init",
            2,
            &json!({
            "error": "depot init cannot ask without a terminal", "missing": ["--pack <name>"]}),
        );
        let refused = yardr.depot("essays", dir.path(), None, "claude", "jo");
        assert_eq!(
            refused.unwrap_err(),
            "yardr asks for something footplate has no answer to: --pack <name>"
        );
        // A dir depot has no base.
        assert!(standin::calls(&yardr.program)[1].starts_with("[depot] [add] [--json]"));

        // What is no JSON is not shown as it came.
        std::fs::write(dir.path().join("answers/up.json"), "<html>").unwrap();
        assert!(
            yardr
                .up()
                .unwrap_err()
                .starts_with("yardr up answered what footplate cannot read")
        );
    }

    /// The same commands, taken by a real yardr: by hand, never in the
    /// gate, and never on a yard a person uses. `FOOTPLATE_REAL_YARDR` is
    /// the yardr to run and `FOOTPLATE_REAL_HOME` an empty directory with a
    /// short path, which becomes the yard's home; the test is run with an
    /// environment that has nothing of another yard in it (`env -i`).
    /// Nothing is started but the yard's own server, which down stops
    /// again: no agent runs, so it asks nothing. `up` and the login are
    /// not in it: they start agents and log in.
    ///
    ///     env -i HOME=<dir> PATH=/usr/bin:/bin FOOTPLATE_REAL_YARDR=<yardr> \
    ///         FOOTPLATE_REAL_HOME=<dir>/.yardr <the test binary> --ignored a_real_yardr
    #[test]
    #[ignore = "runs a real yardr in a home of its own; see the comment"]
    fn a_real_yardr_takes_the_first_run_s_commands() {
        let var = |name: &str| std::env::var_os(name).expect(name);
        assert!(
            std::env::vars().all(|(name, _)| !name.starts_with("YARDR_")),
            "run this with env -i: a YARDR_ variable leads to another yard"
        );
        let home = PathBuf::from(var("FOOTPLATE_REAL_HOME"));
        let mut yardr = Yardr {
            program: PathBuf::from(var("FOOTPLATE_REAL_YARDR")),
            home: Some(home.clone()),
            path: std::env::var_os("PATH").unwrap_or_default(),
            actor: None,
        };
        let doctor = yardr.needs().unwrap();
        assert!(!doctor.yard);
        assert!(doctor.rows().iter().any(|item| item.program == "git"));

        yardr.actor = Some("Jo".to_string());
        let form = Form {
            operator: "Jo".into(),
            harness: "claude".into(),
            yardmaster: "yardmaster".into(),
            model: String::new(),
            style: "guide".into(),
            about: "-a line that starts with a dash".into(),
        };
        let ran = (|| {
            yardr.init(&form)?;
            yardr.init(&form)?;
            // Init stored the style and the line as the operator's
            // settings; setting them again reads back, and an emptied
            // line unsets the note.
            assert_eq!(
                yardr.operator()?,
                OperatorSettings {
                    style: "guide".into(),
                    note: "-a line that starts with a dash".into(),
                }
            );
            yardr.set_operator("report", "")?;
            assert_eq!(
                yardr.operator()?,
                OperatorSettings {
                    style: "report".into(),
                    note: "".into(),
                }
            );
            yardr.set_operator("guide", "-a line that starts with a dash")?;
            let folder = home.parent().unwrap().join("My Essays");
            std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
            let name = depot_name(&folder);
            let base = branch_of(&folder, &yardr.path);
            yardr.depot(&name, &folder, base.as_deref(), "claude", "Jo")?;
            let bead = yardr.bead(&name, "-a first draft", "Done when it reads well.")?;
            assert!(bead.starts_with(&name), "{bead}");
            yardr.subscribe("Jo", &name)?;
            let doctor = yardr.doctor()?;
            assert!(doctor.yard);
            assert_eq!(doctor.yardmaster(), Some("yardmaster"));
            let status = yardr.status().map_err(|err| match err {
                StatusError::NoYard => "the yard is gone".to_string(),
                StatusError::Failed(why) => why,
            })?;
            assert!(status.server_running());
            // No agent runs, so down without the flag stops the yard and
            // asks nothing.
            let down = yardr.down(false)?;
            assert!(down.down && !down.confirm);
            Ok::<_, String>(())
        })();
        // What stopping prints is not JSON, and is not read: the socket
        // being gone is the server stopped.
        let _ = yardr.run(&["server", "stop"], &[]);
        ran.unwrap();
        assert!(!home.join("yard.sock").exists());
    }

    fn next(login: &Login) -> Need {
        login.needs.recv_blocking().unwrap()
    }

    /// A login is led by yardr's JSON alone: the ways as it names them,
    /// the page, the code, the question before the real call, and the
    /// end. What is typed goes to yardr's input and nowhere else.
    #[test]
    fn a_login_is_led_by_the_needs_yardr_prints() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        let mut login = Login::start(&yardr, "claude").unwrap();
        let Need::Choose { options, .. } = next(&login) else {
            panic!("no choice");
        };
        assert_eq!(options[0], "Claude subscription");
        login.choose(&options[0]).unwrap();
        assert_eq!(
            next(&login),
            Need::Page("https://claude.com/cai/oauth/authorize?code=true".into())
        );
        assert!(matches!(next(&login), Need::Input { .. }));
        login.input("good#code".to_string()).unwrap();
        assert!(matches!(next(&login), Need::Confirm { .. }));
        login.confirm(true).unwrap();
        assert_eq!(
            next(&login),
            Need::Done {
                proved: true,
                said: "ok".into()
            }
        );
        assert_eq!(
            standin::answered(&yardr.program),
            [
                r#"{"choose":"Claude subscription"}"#,
                r#"{"input":"good#code"}"#,
                r#"{"confirm":true}"#
            ]
        );
        assert!(standin::calls(&yardr.program)[0].starts_with("[harness] [login] [--json]"));
        assert!(standin::calls(&yardr.program)[0].ends_with("[--] [claude]"));
    }

    /// A code that is none ends the login with yardr's words, and a login
    /// that is left closes yardr's input, which is how yardr stops it.
    #[test]
    fn a_login_that_fails_or_is_left_ends() {
        let dir = tempfile::tempdir().unwrap();
        let yardr = yardr(dir.path());
        let mut login = Login::start(&yardr, "claude").unwrap();
        next(&login);
        login.choose("Claude subscription").unwrap();
        next(&login);
        next(&login);
        login.input("wrong".to_string()).unwrap();
        let Need::Failed(why) = next(&login) else {
            panic!("no failure");
        };
        assert!(why.starts_with("claude auth login failed"), "{why}");

        let left = Login::start(&yardr, "claude").unwrap();
        let needs = left.needs.clone();
        needs.recv_blocking().unwrap();
        drop(left);
        assert_eq!(
            needs.recv_blocking().unwrap(),
            Need::Failed("stdin closed before a method was chosen".into())
        );
    }

    /// A login shell for a test: a script called `name` that runs `script`
    /// whatever it is asked, and keeps what it was asked beside it.
    fn login_shell(dir: &Path, name: &str, script: &str) -> PathBuf {
        let script = format!("#!/bin/sh\nprintf '%s|' \"$@\" >> \"$0.asked\"\n{script}\n");
        crate::fetch::keep(&dir.join("shell"), name, script.as_bytes()).unwrap()
    }

    /// How many times a test's login shell was asked anything.
    fn asks(dir: &Path, name: &str) -> usize {
        let asked = fs::read_to_string(dir.join("shell").join(format!("{name}.asked")));
        asked.unwrap_or_default().matches("-l|-c|").count()
    }

    /// Doctor's answer once herdr and claude are fetched to `home`'s
    /// `.local/bin`, with git where the system has it.
    fn fetched(home: &Path) -> Doctor {
        let bin = home.join(".local/bin");
        serde_json::from_value(json!({
            "yard": false,
            "machine": [
                {"program": "herdr", "found": true, "path": bin.join("herdr")},
                {"program": "git", "found": true, "path": "/usr/bin/git"},
                {"program": "claude", "found": true, "path": bin.join("claude"), "harness": "claude"},
                {"program": "codex", "found": false, "harness": "codex"}
            ]
        }))
        .unwrap()
    }

    /// A shell is known by its program's name, and has the file a terminal
    /// reads it from and the line its syntax wants. zsh's file follows
    /// `ZDOTDIR`; a login bash reads the first of three files, so the
    /// first that is there is its file; any other shell has none, and is
    /// never given one by a guess.
    #[test]
    fn a_shell_has_the_file_and_the_line_that_are_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let known = |program: &str| Shell::of(Some(Path::new(program)));
        assert_eq!(known("/bin/zsh"), Some(Shell::Zsh));
        assert_eq!(known("/usr/local/bin/bash"), Some(Shell::Bash));
        assert_eq!(known("/opt/homebrew/bin/fish"), Some(Shell::Fish));
        for other in ["/bin/tcsh", "/bin/sh", "/usr/bin/nu", "/bin/zsh5"] {
            assert_eq!(known(other), None, "{other}");
        }
        assert_eq!(Shell::of(None), None);

        let posix = r#"export PATH="$HOME/.local/bin:$PATH""#;
        assert_eq!(path_line(Some(Shell::Zsh)), posix);
        assert_eq!(path_line(Some(Shell::Bash)), posix);
        assert_eq!(
            path_line(Some(Shell::Fish)),
            "set -gx PATH $HOME/.local/bin $PATH"
        );
        assert_eq!(path_line(None), posix);

        assert_eq!(startup_file(Shell::Zsh, home, None), home.join(".zshrc"));
        let moved = home.join("dotfiles/zsh");
        assert_eq!(
            startup_file(Shell::Zsh, home, Some(&moved)),
            moved.join(".zshrc")
        );
        // zsh accepts a relative ZDOTDIR; it is relative to the working
        // directory from which zsh starts.
        assert_eq!(
            startup_file(Shell::Zsh, home, Some(Path::new("zsh"))),
            PathBuf::from("zsh/.zshrc")
        );
        assert_eq!(
            startup_file(Shell::Zsh, home, Some(Path::new(""))),
            home.join(".zshrc")
        );
        assert_eq!(
            startup_file(Shell::Fish, home, None),
            home.join(".config/fish/config.fish")
        );

        assert_eq!(
            startup_file(Shell::Bash, home, None),
            home.join(".bash_profile")
        );
        for (made, file) in [
            (".profile", ".profile"),
            (".bash_login", ".bash_login"),
            (".bash_profile", ".bash_profile"),
        ] {
            fs::write(home.join(made), "").unwrap();
            assert_eq!(startup_file(Shell::Bash, home, None), home.join(file));
        }
    }

    /// A file names the directory by a line that holds a PATH and
    /// `.local/bin`, whatever else is in it; a comment does not.
    #[test]
    fn a_line_with_a_path_and_local_bin_names_the_directory() {
        for yes in [
            r#"export PATH="$HOME/.local/bin:$PATH""#,
            "PATH=$PATH:~/.local/bin",
            "  export PATH=/opt/homebrew/bin:$HOME/.local/bin:$PATH  ",
            "set -gx PATH $HOME/.local/bin $PATH",
            "alias a=b\nexport PATH=\"$HOME/.local/bin:$PATH\"\nalias c=d",
            "export PATH=$HOME/.local/bin:$PATH # for pipx",
        ] {
            assert!(names_local_bin(yes), "{yes}");
        }
        for no in [
            "",
            "export PATH=/opt/homebrew/bin:$PATH",
            "alias lb='ls ~/.local/bin'",
            "# export PATH=$HOME/bin:$HOME/.local/bin:/usr/local/bin:$PATH",
            "   #export PATH=\"$HOME/.local/bin:$PATH\"",
        ] {
            assert!(!names_local_bin(no), "{no}");
        }
    }

    /// A directory is on a PATH when one of its entries is that directory,
    /// whatever follows a slash.
    #[test]
    fn a_directory_is_on_a_path_when_it_is_one_of_its_entries() {
        let dir = Path::new("/Users/jo/.local/bin");
        for yes in [
            "/Users/jo/.local/bin",
            "/usr/bin:/Users/jo/.local/bin:/bin",
            "/usr/bin:/Users/jo/.local/bin/",
        ] {
            assert!(on_path(yes.as_ref(), dir), "{yes}");
        }
        for no in [
            "",
            "/usr/bin:/bin",
            "/Users/jo/.local/bin/x:/usr/bin",
            "/Users/jo/.local:/Users/jo/bin",
            "~/.local/bin",
        ] {
            assert!(!on_path(no.as_ref(), dir), "{no}");
        }
    }

    /// Yes adds the line to the end of the file on a line of its own, and
    /// once: a file that has one that names the directory is not touched,
    /// and neither is one that is a link, which the line goes through.
    #[test]
    fn the_line_is_added_once_on_a_line_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let reach = |file: &Path| Reach {
            programs: vec!["herdr".to_string()],
            shell: Some("zsh".to_string()),
            file: Some(file.to_path_buf()),
            line: path_line(Some(Shell::Zsh)),
            home: home.clone(),
        };
        let line = format!("{}\n", path_line(Some(Shell::Zsh)));
        let read = |file: &Path| fs::read_to_string(file).unwrap();

        // A file that is not there is made, with the line alone in it.
        let zshrc = home.join(".zshrc");
        assert_eq!(reach(&zshrc).add(), Ok(Added::Written));
        assert_eq!(read(&zshrc), line);
        // Again, it adds no second.
        assert_eq!(reach(&zshrc).add(), Ok(Added::Named));
        assert_eq!(read(&zshrc), line);

        // A last line without its newline is not joined by the line; one
        // with it gets no blank line.
        let bare = home.join("bare");
        fs::write(&bare, "alias ll='ls -l'").unwrap();
        assert_eq!(reach(&bare).add(), Ok(Added::Written));
        assert_eq!(read(&bare), format!("alias ll='ls -l'\n{line}"));
        let ended = home.join("ended");
        fs::write(&ended, "alias ll='ls -l'\n\n").unwrap();
        assert_eq!(reach(&ended).add(), Ok(Added::Written));
        assert_eq!(read(&ended), format!("alias ll='ls -l'\n\n{line}"));

        // A file that names the directory is left byte for byte, and its
        // time as it was.
        let named = home.join("named");
        fs::write(&named, "export PATH=$PATH:~/.local/bin").unwrap();
        let before = fs::metadata(&named).unwrap().modified().unwrap();
        assert_eq!(reach(&named).add(), Ok(Added::Named));
        assert_eq!(read(&named), "export PATH=$PATH:~/.local/bin");
        assert_eq!(fs::metadata(&named).unwrap().modified().unwrap(), before);

        // A comment is no PATH entry.
        let commented = home.join("commented");
        fs::write(&commented, "# export PATH=$HOME/.local/bin:$PATH\n").unwrap();
        assert_eq!(reach(&commented).add(), Ok(Added::Written));
        assert_eq!(
            read(&commented),
            format!("# export PATH=$HOME/.local/bin:$PATH\n{line}")
        );

        // A link stays one, and what it leads to has the line.
        let target = home.join("dotfiles/zshrc");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, "alias ll='ls -l'\n").unwrap();
        let link = home.join("linked");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(reach(&link).add(), Ok(Added::Written));
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(read(&target), format!("alias ll='ls -l'\n{line}"));
        assert_eq!(reach(&link).add(), Ok(Added::Named));

        // The directory of fish's file is made.
        let fish = home.join(".config/fish/config.fish");
        let mut reach_fish = reach(&fish);
        reach_fish.line = path_line(Some(Shell::Fish));
        assert_eq!(reach_fish.add(), Ok(Added::Written));
        assert_eq!(read(&fish), "set -gx PATH $HOME/.local/bin $PATH\n");
    }

    /// A file that cannot be read, or cannot be written, is said so in the
    /// system's words, and nothing is added.
    #[test]
    fn a_file_that_cannot_be_read_or_written_says_so_and_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let reach = |file: PathBuf| Reach {
            programs: vec!["herdr".to_string()],
            shell: Some("zsh".to_string()),
            file: Some(file),
            line: path_line(Some(Shell::Zsh)),
            home: home.clone(),
        };

        // A folder where the file would be cannot be read.
        let folder = home.join(".zshrc");
        fs::create_dir(&folder).unwrap();
        let said = reach(folder.clone()).add().unwrap_err();
        assert!(
            said.starts_with("~/.zshrc could not be read (")
                && said.ends_with(" Nothing was added."),
            "{said}"
        );
        assert_eq!(fs::read_dir(&folder).unwrap().count(), 0);

        // A file the person's account may not write: only without root,
        // which no mode holds.
        if crate::yard::running_as_root() {
            return;
        }
        let locked = home.join("locked");
        fs::write(&locked, "alias ll='ls -l'\n").unwrap();
        fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o444)).unwrap();
        let said = reach(locked.clone()).add().unwrap_err();
        assert!(
            said.starts_with("~/locked could not be written (")
                && said.ends_with(" Nothing was added."),
            "{said}"
        );
        assert_eq!(fs::read_to_string(&locked).unwrap(), "alias ll='ls -l'\n");
    }

    /// A shell that is not known has no file: yes writes nothing for it,
    /// in words that say so, and no file or folder is made.
    #[test]
    fn a_shell_that_is_not_known_is_shown_the_line_and_written_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let doctor = fetched(&home);
        let yardr = home.join(".local/bin/yardr");
        let machine = |shell: &Path| Machine {
            home: Some(home.clone()),
            shell: Some(shell.to_path_buf()),
            within: Duration::from_secs(60),
            ..Machine::default()
        };

        let tcsh = login_shell(dir.path(), "tcsh", "echo /usr/bin:/bin");
        let reach_tcsh = reach(&machine(&tcsh), &yardr, &doctor).unwrap();
        assert_eq!(reach_tcsh.file, None);
        assert_eq!(reach_tcsh.shell.as_deref(), Some("tcsh"));
        assert_eq!(reach_tcsh.line, r#"export PATH="$HOME/.local/bin:$PATH""#);
        assert_eq!(
            reach_tcsh.add(),
            Err(
                "footplate does not know where tcsh keeps its startup file, so it wrote nothing."
                    .to_string()
            )
        );
        // None named at all.
        let none = reach(
            &Machine {
                shell: None,
                ..machine(&tcsh)
            },
            &yardr,
            &doctor,
        )
        .unwrap();
        assert_eq!((none.file.clone(), none.shell.clone()), (None, None));
        assert_eq!(
            none.add(),
            Err(
                "footplate does not know which shell a terminal runs here, so it wrote nothing."
                    .to_string()
            )
        );
        // Nothing was made in the home.
        assert_eq!(fs::read_dir(&home).unwrap().count(), 0);
    }

    /// Setup has something to do for a terminal where a program is in
    /// `~/.local/bin` and neither the file of the person's shell nor the
    /// PATH its login shell reports names the directory. The file is asked
    /// first, and is what stops a second step after a yes: a login shell
    /// started to answer `-l -c` does not read zsh's `.zshrc`, so it says
    /// what it said before.
    #[test]
    fn a_terminal_is_given_a_step_only_where_nothing_names_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let (yardr, doctor) = (bin.join("yardr"), fetched(&home));
        let machine = |shell: &Path| Machine {
            home: Some(home.clone()),
            shell: Some(shell.to_path_buf()),
            within: Duration::from_secs(60),
            ..Machine::default()
        };
        let zsh = login_shell(dir.path(), "zsh", "echo /usr/bin:/bin");
        let zshrc = home.join(".zshrc");

        // yardr, herdr and claude are there, and a terminal does not look
        // in the directory: the step, with the file and the line.
        let step = reach(&machine(&zsh), &yardr, &doctor).unwrap();
        assert_eq!(step.programs, ["yardr", "herdr", "claude"]);
        assert_eq!(step.named(), "yardr, herdr and claude");
        assert_eq!(step.file, Some(zshrc.clone()));
        assert_eq!(step.shown().as_deref(), Some("~/.zshrc"));
        assert_eq!(step.line, r#"export PATH="$HOME/.local/bin:$PATH""#);
        assert_eq!(asks(dir.path(), "zsh"), 1);
        assert!(!zshrc.exists(), "asking writes nothing");

        // Yes. A second run has no step, though the login shell still says
        // what it said: the file does, and the shell is not even asked.
        assert_eq!(step.add(), Ok(Added::Written));
        assert_eq!(reach(&machine(&zsh), &yardr, &doctor), None);
        assert_eq!(asks(dir.path(), "zsh"), 1);
        assert_eq!(
            fs::read_to_string(&zshrc).unwrap(),
            format!("{}\n", step.line)
        );
        fs::remove_file(&zshrc).unwrap();

        // The login shell's PATH names it already: no step, and no file.
        let on = login_shell(
            dir.path(),
            "zsh",
            &format!("echo /usr/bin:{}", bin.display()),
        );
        assert_eq!(reach(&machine(&on), &yardr, &doctor), None);
        assert!(!zshrc.exists());

        // A shell that fails or takes too long has not said it is there.
        let failing = login_shell(dir.path(), "zsh", "exit 1");
        assert!(reach(&machine(&failing), &yardr, &doctor).is_some());

        // A comment is no entry.
        fs::write(&zshrc, "# export PATH=$HOME/bin:$HOME/.local/bin:$PATH\n").unwrap();
        assert!(reach(&machine(&zsh), &yardr, &doctor).is_some());
        fs::remove_file(&zshrc).unwrap();

        // No program of setup's is in the directory: nothing to do, and the
        // shell is not asked. The yardr elsewhere, herdr and claude found
        // elsewhere, one not found with a path doctor does not vouch for.
        let before = asks(dir.path(), "zsh");
        let elsewhere: Doctor = serde_json::from_value(json!({"machine": [
            {"program": "herdr", "found": true, "path": "/opt/stand-in/bin/herdr"},
            {"program": "claude", "found": false, "path": bin.join("claude")},
        ]}))
        .unwrap();
        assert_eq!(
            reach(
                &machine(&zsh),
                Path::new("/opt/yardr/bin/yardr"),
                &elsewhere
            ),
            None
        );
        assert_eq!(asks(dir.path(), "zsh"), before);
        // Only the yardr is: it is the step's one program.
        let alone = reach(&machine(&zsh), &yardr, &elsewhere).unwrap();
        assert_eq!(alone.named(), "yardr");

        // Doctor can name a found harness under its "a harness" item.
        // It is still one of the programs this step tells the person about.
        let nested: Doctor = serde_json::from_value(json!({"machine": [
            {"program": "a harness", "found": false, "any_of": [
                {"program": "codex", "harness": "codex", "found": true,
                 "path": bin.join("codex")}
            ]}
        ]}))
        .unwrap();
        let nested = reach(&machine(&zsh), &yardr, &nested).unwrap();
        assert_eq!(nested.programs, ["yardr", "codex"]);

        // bash writes the file a login bash reads: a `.profile` that names
        // it makes no step, and no `.bash_profile` is made to hide it.
        let bash = login_shell(dir.path(), "bash", "echo /usr/bin:/bin");
        fs::write(
            home.join(".profile"),
            "export PATH=\"$HOME/.local/bin:$PATH\"\n",
        )
        .unwrap();
        assert_eq!(reach(&machine(&bash), &yardr, &doctor), None);
        assert!(!home.join(".bash_profile").exists());
        fs::write(home.join(".profile"), "alias ll='ls -l'\n").unwrap();
        let step = reach(&machine(&bash), &yardr, &doctor).unwrap();
        assert_eq!(step.file, Some(home.join(".profile")));
        assert_eq!(step.add(), Ok(Added::Written));
        assert!(!home.join(".bash_profile").exists());
        assert_eq!(reach(&machine(&bash), &yardr, &doctor), None);

        // zsh keeps its files in ZDOTDIR where the environment says so.
        let moved = dir.path().join("dotfiles/zsh");
        let step = reach(
            &Machine {
                zdotdir: Some(moved.clone()),
                ..machine(&zsh)
            },
            &yardr,
            &doctor,
        )
        .unwrap();
        assert_eq!(step.file, Some(moved.join(".zshrc")));
        assert_eq!(step.add(), Ok(Added::Written));
        assert!(moved.join(".zshrc").exists() && !zshrc.exists());
    }

    /// Only a page on the web is handed to the system's opener.
    #[test]
    fn only_an_https_address_is_a_page() {
        assert!(is_page("https://claude.com/cai/oauth/authorize?x=1"));
        for no in [
            "http://claude.com",
            "file:///etc/passwd",
            "https://",
            "https://a b",
            "javascript:alert(1)",
            "https://a\nb",
        ] {
            assert!(!is_page(no), "{no}");
        }
        assert_eq!(
            Need::read(r#"{"need":"open_url","url":"file:///etc/passwd"}"#),
            None
        );
    }

    /// The base branch of a folder is the one it has checked out, and a
    /// folder that is no checkout has none.
    #[test]
    fn a_folder_s_branch_is_read_from_git_and_a_plain_folder_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::var_os("PATH").unwrap_or_default();
        assert_eq!(branch_of(dir.path(), &path), None);
        let init = Command::new("git")
            .args(["init", "-q", "-b", "trunk"])
            .arg(dir.path())
            .status();
        if init.is_ok_and(|status| status.success()) {
            assert_eq!(branch_of(dir.path(), &path).as_deref(), Some("trunk"));
        }
        assert_eq!(depot_name(Path::new("/Users/jo/My Essays!")), "my-essays");
        assert_eq!(depot_name(Path::new("/")), "");
    }
}
