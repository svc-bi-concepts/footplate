//! "Set up a yard here": the first run of a yard on this machine, from no
//! yard to talking with the yardmaster, with no terminal. It stands where
//! the window says there is no yard, as the third way to one, and each of
//! its steps is a command of the yardr on this machine (setup.rs):
//! what yardr answers is what is shown, in yardr's words where it refuses.
//!
//! The steps, in order: yardr itself is found or installed (fetch.rs); what
//! the machine lacks is listed from `yardr doctor --json`, and installed
//! where yardr names a way; the yard is created (`yardr init`); the harness
//! is logged in, led by `yardr harness login --json`; the yard is started
//! (`yardr up`); a folder becomes its first depot, with a first bead. Then
//! the window opens the yardmaster's conversation.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, AnyWindowHandle, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, MouseButton, PathPromptOptions, SharedString, Task, Window, div, prelude::*, rems,
};

use super::small_button;
use crate::feedback::Project;
use crate::fetch::{self, Machine};
use crate::input::{Line, Submitted};
use crate::setup::{self, Doctor, Form, Item, Login, Need, Offer, Yardr};
use crate::theme::Palette;
use crate::titlebar::{self, Gesture};

/// Gives the yardr to run: where it is, and what there is to say of it
/// where the one an older footplate kept could not be moved. In a sentence,
/// why there is none.
pub type Finds = Arc<dyn Fn() -> Result<(PathBuf, Option<String>), String> + Send + Sync>;

/// Runs yardr's installer, sending what it prints as it goes, and gives the
/// yardr it left. In a sentence, why there is none.
pub type Installs =
    Arc<dyn Fn(Option<async_channel::Sender<String>>) -> Result<PathBuf, String> + Send + Sync>;

/// Opens a page on the web in the person's browser.
pub type Opens = Rc<dyn Fn(&str, &mut App)>;

/// What the window does for a gesture on its top strip: moves, or zooms.
pub type Strip = Rc<dyn Fn(Gesture, &mut Window, &mut App)>;

/// What the first run, the window's top strip and its Feedback form reach
/// outside the window with. A test has a stand-in for each.
#[derive(Clone)]
pub struct Means {
    pub yardr: Finds,
    pub install: Installs,
    /// What yardr is looked for by, and where a program that is fetched
    /// goes: the person's home, and footplate's own directory for programs.
    pub machine: Machine,
    pub open: Opens,
    /// This is a Mac: git comes with Apple's own installer.
    pub apple: bool,
    /// Where the window's content reaches up into its title bar, as on a
    /// Mac, what the window does for a gesture on its top strip. None where
    /// the desktop draws a title bar above the window, and in a test, whose
    /// window has none.
    pub titlebar: Option<Strip>,
    /// Where feedback goes, if it is offered: yardr's PostHog project, none
    /// while its survey is. A form is never offered that sends nowhere.
    pub feedback: Option<Project>,
}

impl Means {
    /// This machine's.
    pub fn real() -> Means {
        // Tests look at no machine, and fetch nothing: a test that reaches
        // out for yardr stands in for it.
        #[cfg(not(test))]
        let machine = Machine::here();
        #[cfg(test)]
        let machine = Machine::default();
        Means {
            yardr: {
                let machine = machine.clone();
                Arc::new(move || machine.yardr())
            },
            install: {
                let machine = machine.clone();
                Arc::new(move |progress| machine.install(progress))
            },
            machine,
            open: Rc::new(|url, cx| cx.open_url(url)),
            apple: cfg!(target_os = "macos"),
            titlebar: (cfg!(target_os = "macos") && !cfg!(test))
                .then(|| Rc::new(titlebar::perform) as Strip),
            // Tests send nothing: a test that sends feedback stands in for
            // PostHog.
            feedback: Project::yardr().filter(|_| !cfg!(test)),
        }
    }
}

/// Where the first run stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    /// yardr itself is looked for, and installed where it is missing.
    Yardr,
    /// What the machine has of what a yard needs.
    Needs,
    /// The yard to create.
    Yard,
    /// The harness's login.
    Login,
    /// Starting the yard.
    Start,
    /// A folder for the first depot.
    Depot,
    /// The first bead.
    Bead,
}

impl Step {
    fn title(self) -> &'static str {
        match self {
            Step::Yardr => "yardr itself",
            Step::Needs => "What this needs",
            Step::Yard => "Your yard",
            Step::Login => "Logging in",
            Step::Start => "Start the yard",
            Step::Depot => "A folder to work in",
            Step::Bead => "A first piece of work",
        }
    }

    fn number(self) -> usize {
        match self {
            Step::Yardr | Step::Needs => 1,
            Step::Yard => 2,
            Step::Login => 3,
            Step::Start => 4,
            Step::Depot | Step::Bead => 5,
        }
    }
}

/// How the first run ended.
pub enum Ended {
    /// The yard runs: the yardmaster, by its name, is whom to talk with.
    Talking { yardmaster: String },
    /// The person left it. What was made stays as it is.
    Left,
}

pub struct FirstRun {
    means: Means,
    /// The yard's directory: where this run's socket is.
    home: PathBuf,
    pub step: Step,
    /// What is underway, in a few words: nothing else is started
    /// meanwhile.
    pub busy: Option<String>,
    /// What came of the last thing, where it did not come about, or what
    /// there is to know.
    pub said: Option<String>,
    yardr: Option<Yardr>,
    doctor: Option<Doctor>,
    /// The home has a yard already, which is not running: nothing of it
    /// is created again, and it is started.
    existing: bool,
    /// The yard as it was created: the commands after it are its
    /// operator's.
    form: Form,
    operator: Entity<Line>,
    yardmaster: Entity<Line>,
    model: Entity<Line>,
    about: Entity<Line>,
    /// The harness and the style chosen in the form.
    harness: Option<String>,
    /// The model picked from the chosen harness's offers: None is the
    /// harness's own choice. Choosing a harness resets it to that
    /// harness's recommended offer, and a typed model id wins over it.
    model_pick: Option<String>,
    style: usize,
    /// The login that is underway, what it needs now, and the page it
    /// named. The field's text is what the page gave: it goes to yardr and
    /// is in the field no longer.
    login: Option<Login>,
    need: Option<Need>,
    page: Option<String>,
    code: Entity<Line>,
    _leading: Option<Task<()>>,
    folder: Entity<Line>,
    depot: Entity<Line>,
    base: Entity<Line>,
    /// The depot that was set up, which the first bead is of.
    made: String,
    title: Entity<Line>,
    goal: Entity<Line>,
    /// Whether the person is told when a bead lands.
    tell: bool,
    focus: FocusHandle,
    /// The window this is drawn in: a field the login asks for has the
    /// keys when it comes, which is with no key or click of its own.
    handle: AnyWindowHandle,
}

impl EventEmitter<Ended> for FirstRun {}

impl Focusable for FirstRun {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FirstRun {
    /// The first run of the yard at `socket`, for the person whose login
    /// is `name`. It starts by looking for yardr.
    pub fn new(
        means: Means,
        socket: &Path,
        name: &str,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(means, socket, name, handle, cx);
        this.find(cx);
        this
    }

    /// Step 2 posed for a screenshot: the yard's form with a provider
    /// chosen, its model offers to pick from, and the scale to pick from.
    /// A debug build's alone, and a unix one's: nothing is fetched, and no
    /// command runs.
    #[cfg(all(debug_assertions, unix))]
    pub fn shot_step2(
        means: Means,
        socket: &Path,
        name: &str,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut this = Self::build(means, socket, name, handle, cx);
        this.doctor = Some(crate::shot::setup_doctor());
        let harness = this.doctor.as_ref().and_then(|doctor| {
            doctor
                .installed()
                .first()
                .and_then(|item| item.harness.clone())
        });
        this.model_pick = harness.as_deref().and_then(|kind| this.recommended(kind));
        this.harness = harness;
        this.step = Step::Yard;
        this
    }

    /// The first run before it starts: its fields, with the login name
    /// standing in the operator's.
    fn build(
        means: Means,
        socket: &Path,
        name: &str,
        handle: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = |hint: &'static str, text: &str, cx: &mut Context<Self>| {
            let field = cx.new(|cx| {
                let mut field = Line::new(cx);
                field.set_placeholder(hint, cx);
                field.set_text(text, cx);
                field
            });
            // Enter in a field is the step's own button.
            cx.subscribe(&field, |this, _, _: &Submitted, cx| this.submit(cx))
                .detach();
            field
        };
        let code = cx.new(Line::masked);
        cx.subscribe(&code, |this, _, _: &Submitted, cx| this.give_code(cx))
            .detach();
        FirstRun {
            means,
            home: socket.parent().map(Path::to_path_buf).unwrap_or_default(),
            step: Step::Yardr,
            busy: None,
            said: None,
            yardr: None,
            doctor: None,
            existing: false,
            form: Form::default(),
            operator: field("Jo", name, cx),
            yardmaster: field("yardmaster", "yardmaster", cx),
            model: field("the pick above stands", "", cx),
            about: field("I teach history and do not read code", "", cx),
            harness: None,
            model_pick: None,
            style: 0,
            login: None,
            need: None,
            page: None,
            code,
            _leading: None,
            folder: field("/Users/jo/essays", "", cx),
            depot: field("essays", "", cx),
            base: field("main", "", cx),
            made: String::new(),
            title: field("A first draft of the essay on 1848", "", cx),
            goal: field(
                "Done when it reads well aloud and is under 800 words.",
                "",
                cx,
            ),
            tell: true,
            focus: cx.focus_handle(),
            handle,
        }
    }

    /// Does `work` away from the window and hands what it came to to
    /// `then`, with `busy` shown meanwhile. Nothing is started while
    /// something is underway.
    fn work<T: Send + 'static>(
        &mut self,
        busy: &str,
        work: impl FnOnce() -> T + Send + 'static,
        then: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(busy.to_string());
        self.said = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let came = cx.background_executor().spawn(async move { work() }).await;
            this.update(cx, |this, cx| {
                this.busy = None;
                then(this, came, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Step 0: the yardr on this machine.
    fn find(&mut self, cx: &mut Context<Self>) {
        self.step = Step::Yardr;
        let find = self.means.yardr.clone();
        self.work(
            "looking for yardr…",
            move || find(),
            |this, found, cx| match found {
                Ok((program, note)) => {
                    this.use_yardr(program, cx);
                    // After the check began: it is what there is to know
                    // while the rows are read.
                    this.said = note;
                }
                Err(why) => this.said = Some(why),
            },
            cx,
        );
    }

    /// Step 0 where there is none: yardr's own installer, on the person's
    /// word, showing what it prints.
    fn install_yardr(&mut self, cx: &mut Context<Self>) {
        let install = self.means.install.clone();
        let installing = "installing yardr…".to_string();
        let (send, progress) = async_channel::bounded(1);
        self.work(
            &installing,
            move || install(Some(send)),
            |this, installed, cx| match installed {
                Ok(program) => this.use_yardr(program, cx),
                Err(why) => this.said = Some(why),
            },
            cx,
        );
        self.follow(progress, installing, cx);
    }

    fn use_yardr(&mut self, program: PathBuf, cx: &mut Context<Self>) {
        self.yardr = Some(Yardr {
            program,
            home: Some(self.home.clone()),
            path: self.means.machine.path.clone(),
            actor: None,
        });
        self.check(cx);
    }

    /// Step 1: what the machine has, as doctor says it.
    fn check(&mut self, cx: &mut Context<Self>) {
        let Some(mut yardr) = self.yardr.clone() else {
            return;
        };
        self.step = Step::Needs;
        self.work(
            "looking at what this machine has…",
            move || {
                let doctor = yardr.needs();
                (yardr, doctor)
            },
            |this, (yardr, doctor), _| {
                this.yardr = Some(yardr);
                match doctor {
                    Ok(doctor) => this.doctor = Some(doctor),
                    Err(why) => this.said = Some(why),
                }
            },
            cx,
        );
    }

    /// Installs the program of doctor's row `program` the way yardr names
    /// for it, and looks again. git on a Mac is Apple's own installer,
    /// which shows its dialog and is waited for by nobody.
    fn install(&mut self, program: &str, cx: &mut Context<Self>) {
        let (Some(yardr), Some(doctor)) = (self.yardr.clone(), self.doctor.as_ref()) else {
            return;
        };
        if program == "git" && self.means.apple {
            self.said = Some(match setup::apple_tools() {
                Ok(()) => "Apple's installer is asked for git: it shows its own window. When it is done, check again.".to_string(),
                Err(why) => why,
            });
            cx.notify();
            return;
        }
        // Doctor's fetch data is a Mac installer road. Linux must stay a
        // list with the maker's page and yardr's instructions, even if a
        // later yardr release accidentally includes that data there.
        if !self.means.apple {
            return;
        }
        let wanted = doctor
            .rows()
            .into_iter()
            .find(|item| item.program == program)
            .and_then(|item| item.fetch.clone());
        let (Some(wanted), Some(home), Some(own)) = (
            wanted,
            self.means.machine.home.clone(),
            self.means.machine.own.clone(),
        ) else {
            return;
        };
        let host = wanted.url.split('/').nth(2).unwrap_or_default().to_string();
        let fetching = format!("fetching {program} {} from {host}…", wanted.version);
        let installing = format!("installing {program}…");
        let (send, progress) = async_channel::bounded(1);
        self.work(
            &fetching,
            move || fetch::program_with_progress(&wanted, &home, &own, &yardr.path, Some(send)),
            |this, fetched, cx| match fetched {
                Ok(()) => this.check(cx),
                Err(why) => this.said = Some(why),
            },
            cx,
        );
        self.follow(progress, installing, cx);
    }

    /// Shows what an installer prints as it runs, behind `installing`.
    fn follow(
        &self,
        progress: async_channel::Receiver<String>,
        installing: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Ok(line) = progress.recv().await {
                if this
                    .update(cx, |this, cx| {
                        let is_installing = this.busy.as_deref().is_some_and(|busy| {
                            busy.starts_with("fetching ") || busy.starts_with(&installing)
                        });
                        if is_installing {
                            this.busy = Some(format!("{installing} {line}"));
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Opens a page on the web: an `https://` address and nothing else.
    fn open(&mut self, url: &str, cx: &mut Context<Self>) {
        if setup::is_page(url) {
            (self.means.open.clone())(url, cx);
        }
    }

    /// Step 2 is shown once git, herdr and a harness are there.
    fn go_on(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doctor) = self.doctor.as_ref().filter(|doctor| doctor.ready()) else {
            return;
        };
        // A yard that is there is not created over: what its form would
        // say is the yard's own already.
        if doctor.yard {
            self.existing = true;
            self.step = Step::Start;
            self.said = Some("This machine has a yard already, which is not running.".to_string());
            cx.notify();
            return;
        }
        let harness = doctor
            .installed()
            .first()
            .and_then(|item| item.harness.clone());
        self.model_pick = harness.as_deref().and_then(|kind| self.recommended(kind));
        self.harness = harness;
        self.step = Step::Yard;
        self.said = None;
        window.focus(&self.operator.focus_handle(cx));
        cx.notify();
    }

    /// The step's own button, which Enter in a field of it is.
    fn submit(&mut self, cx: &mut Context<Self>) {
        match self.step {
            Step::Yard => self.create(cx),
            Step::Depot => self.set_up(cx),
            Step::Bead => self.first_bead(cx),
            Step::Yardr | Step::Needs | Step::Login | Step::Start => {}
        }
    }

    /// Creates the yard that is typed, and starts nothing.
    fn create(&mut self, cx: &mut Context<Self>) {
        let (Some(mut yardr), Some(harness)) = (self.yardr.clone(), self.harness.clone()) else {
            return;
        };
        let typed = |line: &Entity<Line>| line.read(cx).text().trim().to_string();
        let form = Form {
            operator: typed(&self.operator),
            harness,
            yardmaster: match typed(&self.yardmaster).as_str() {
                "" => "yardmaster".to_string(),
                name => name.to_string(),
            },
            model: match typed(&self.model) {
                overrode if !overrode.is_empty() => overrode,
                _ => self.model_pick.clone().unwrap_or_default(),
            },
            style: setup::STYLES[self.style].0.to_string(),
            about: typed(&self.about),
        };
        if form.operator.is_empty() {
            self.said = Some("Your name is wanted: it is what the agents call you.".to_string());
            cx.notify();
            return;
        }
        // From here on the commands are the operator's, whatever the
        // login is called.
        yardr.actor = Some(form.operator.clone());
        let (asked, made) = (form.clone(), yardr.clone());
        self.work(
            "creating the yard…",
            move || yardr.init(&asked),
            move |this, created, _| match created {
                Ok(()) => {
                    this.yardr = Some(made);
                    this.form = form;
                    this.step = Step::Login;
                }
                Err(why) => this.said = Some(why),
            },
            cx,
        );
    }

    /// The installed harness of `kind`, as doctor spoke of it.
    fn installed_item(&self, kind: &str) -> Option<&Item> {
        self.doctor
            .as_ref()?
            .installed()
            .into_iter()
            .find(|item| item.harness.as_deref() == Some(kind))
    }

    /// The model id the pick resets to for `kind`: its offer recommended
    /// for a yardmaster, else the harness's own choice.
    fn recommended(&self, kind: &str) -> Option<String> {
        self.installed_item(kind)?
            .recommended_offer()
            .map(|offer| offer.model.clone())
    }

    /// The text of a provider's button: the maker and the program where
    /// doctor carries both ("Anthropic · Claude Code"), the one it
    /// carries where it carries one ("Claude Code", or the maker with
    /// the kind: "Anthropic · claude"), else the kind. One with no
    /// login recipe says it needs a terminal once.
    fn provider_text(item: &Item) -> String {
        let kind = item.harness.as_deref().unwrap_or_default();
        let provider = item.provider.as_deref().filter(|name| !name.is_empty());
        let label = item.label.as_deref().filter(|name| !name.is_empty());
        let name = match (provider, label) {
            (Some(provider), Some(label)) => format!("{provider} · {label}"),
            (None, Some(label)) => label.to_string(),
            (Some(provider), None) => format!("{provider} · {kind}"),
            (None, None) => kind.to_string(),
        };
        match item.harness_login {
            true => name,
            false => format!("{name} (needs a terminal once to log in)"),
        }
    }

    /// The text of an offer's button: its label, the yardmaster's pick
    /// marked.
    fn offer_text(offer: &Offer) -> String {
        let name = match offer.label.is_empty() {
            true => offer.model.as_str(),
            false => offer.label.as_str(),
        };
        match offer.for_yardmaster() {
            true => format!("{name} (recommended)"),
            false => name.to_string(),
        }
    }

    /// Whether yardr can lead the login of the yard's harness.
    fn login_led(&self) -> bool {
        self.doctor.as_ref().is_some_and(|doctor| {
            doctor.harnesses().iter().any(|item| {
                item.harness.as_deref() == Some(self.form.harness.as_str()) && item.harness_login
            })
        })
    }

    /// Starts the login yardr leads, and shows each thing it needs as it
    /// asks for it.
    fn log_in(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.yardr.as_ref() else {
            return;
        };
        if self.busy.is_some() || self.login.is_some() {
            return;
        }
        self.said = None;
        self.need = None;
        self.page = None;
        cx.notify();
        let login = match Login::start(yardr, &self.form.harness) {
            Ok(login) => login,
            Err(why) => {
                self.said = Some(why);
                return;
            }
        };
        let needs = login.needs.clone();
        self.login = Some(login);
        self._leading = Some(cx.spawn(async move |this, cx| {
            while let Ok(need) = needs.recv().await {
                let went = this.update(cx, |this, cx| this.needed(need, cx));
                if went.is_err() {
                    break;
                }
            }
        }));
    }

    /// The login needs `need` now, or is over.
    fn needed(&mut self, need: Need, cx: &mut Context<Self>) {
        cx.notify();
        match need {
            Need::Page(page) => self.page = Some(page),
            Need::Done { proved, said } => {
                self.leave_login(cx);
                self.step = Step::Start;
                self.said = Some(match (proved, said.is_empty()) {
                    (true, _) => "Logged in, and the harness answers.".to_string(),
                    (false, true) => "Logged in.".to_string(),
                    (false, false) => format!("Logged in, but the harness did not answer: {said}"),
                });
            }
            Need::Failed(why) => {
                self.leave_login(cx);
                self.said = Some(why);
            }
            need => {
                if matches!(need, Need::Input { .. }) {
                    // Afterwards: the window is not to be had from here.
                    let (handle, focus) = (self.handle, self.code.focus_handle(cx));
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, _| window.focus(&focus));
                    });
                }
                self.need = Some(need);
            }
        }
    }

    /// The login is over, or left: yardr's input closes, and nothing of
    /// what was typed for it stays in the field.
    fn leave_login(&mut self, cx: &mut Context<Self>) {
        self.login = None;
        self._leading = None;
        self.need = None;
        self.page = None;
        self.code.update(cx, |line, cx| line.set_text("", cx));
    }

    /// An answer to the login that is underway.
    fn answer(
        &mut self,
        answer: impl FnOnce(&mut Login) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(login) = self.login.as_mut() else {
            return;
        };
        // What was asked is answered: the next thing the login needs
        // takes its place, and a page stays for as long as it is good.
        self.need = None;
        if let Err(why) = answer(login) {
            self.said = Some(why);
        }
        cx.notify();
    }

    /// Enter in the code's field: what the page gave goes to yardr, and
    /// is in the field no longer.
    fn give_code(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.need, Some(Need::Input { .. })) {
            return;
        }
        let typed = self.code.update(cx, |line, cx| line.take_text(cx));
        if typed.trim().is_empty() {
            return;
        }
        self.answer(|login| login.input(typed.trim().to_string()), cx);
    }

    /// The harness is logged in already, or is logged in in a terminal:
    /// on to starting.
    fn skip_login(&mut self, cx: &mut Context<Self>) {
        self.leave_login(cx);
        self.step = Step::Start;
        self.said = None;
        cx.notify();
    }

    /// Step 4: `yardr up`. What could not be started is said with the one
    /// thing to do, in yardr's words.
    fn start(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.yardr.clone() else {
            return;
        };
        let existing = self.existing;
        self.work(
            "starting the yard…",
            move || {
                let up = yardr.up();
                // A yard that was there has its own yardmaster, whose name
                // doctor reads once the yard answers.
                let named = match &up {
                    Ok(up) if up.up && existing => yardr
                        .doctor()
                        .ok()
                        .and_then(|doctor| doctor.yardmaster().map(str::to_string)),
                    _ => None,
                };
                (up, named)
            },
            move |this, (up, named), cx| match up {
                Ok(up) if up.up && existing => {
                    this.form.yardmaster = named.unwrap_or_else(|| "yardmaster".to_string());
                    this.talk(cx);
                }
                Ok(up) if up.up => this.step = Step::Depot,
                Ok(up) => {
                    this.said = Some(match up.troubles() {
                        troubles if troubles.is_empty() => {
                            "The yard did not start, and yardr did not say why.".to_string()
                        }
                        troubles => troubles.join("\n"),
                    })
                }
                Err(why) => this.said = Some(why),
            },
            cx,
        );
    }

    /// Asks the system for a folder, and puts it in the field with the
    /// name and the branch it leads to.
    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = chosen.await else {
                return;
            };
            let Some(dir) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |this, cx| this.folder_is(&dir, cx)).ok();
        })
        .detach();
    }

    /// The folder is `dir`: the name and the base branch are offered for
    /// it, each to be changed.
    fn folder_is(&mut self, dir: &Path, cx: &mut Context<Self>) {
        let text = dir.to_string_lossy().into_owned();
        self.folder.update(cx, |line, cx| line.set_text(&text, cx));
        let name = setup::depot_name(dir);
        self.depot.update(cx, |line, cx| line.set_text(&name, cx));
        let path = self.yardr.as_ref().map(|yardr| yardr.path.clone());
        let branch = setup::branch_of(dir, &path.unwrap_or_default()).unwrap_or_default();
        self.base.update(cx, |line, cx| line.set_text(&branch, cx));
        cx.notify();
    }

    /// Step 5: the folder becomes a depot, set up from the yard's pack.
    /// A folder that is no git checkout is a depot of the plain kind.
    fn set_up(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.yardr.clone() else {
            return;
        };
        let typed = |line: &Entity<Line>| line.read(cx).text().trim().to_string();
        let (folder, name, base) = (typed(&self.folder), typed(&self.depot), typed(&self.base));
        let dir = PathBuf::from(&folder);
        if !dir.is_absolute() || !dir.is_dir() {
            self.said = Some(match folder.is_empty() {
                true => "A folder is wanted: the one the work is in.".to_string(),
                false => format!("{folder} is no folder on this machine."),
            });
            cx.notify();
            return;
        }
        let (harness, operator) = (self.form.harness.clone(), self.form.operator.clone());
        self.work(
            "setting the folder up…",
            move || {
                let name = match name.is_empty() {
                    true => setup::depot_name(&dir),
                    false => name,
                };
                // The branch that is typed, else the one the folder has
                // checked out; a folder that is no checkout has none.
                let checkout = setup::branch_of(&dir, &yardr.path);
                let base = match (checkout, base.is_empty()) {
                    (None, _) => None,
                    (Some(_), false) => Some(base),
                    (Some(branch), true) => Some(branch),
                };
                yardr
                    .depot(&name, &dir, base.as_deref(), &harness, &operator)
                    .map(|()| name)
            },
            |this, made, _| match made {
                Ok(name) => {
                    this.made = name;
                    this.step = Step::Bead;
                }
                Err(why) => this.said = Some(why),
            },
            cx,
        );
    }

    /// The first bead, handed to the builders, and the person told when
    /// beads land where they asked for it. Then the yardmaster.
    fn first_bead(&mut self, cx: &mut Context<Self>) {
        let Some(yardr) = self.yardr.clone() else {
            return;
        };
        let typed = |line: &Entity<Line>| line.read(cx).text().trim().to_string();
        let (title, goal) = (typed(&self.title), typed(&self.goal));
        if title.is_empty() {
            self.said =
                Some("The work wants a title: a line that says what is to be done.".to_string());
            cx.notify();
            return;
        }
        let (depot, operator, tell) = (self.made.clone(), self.form.operator.clone(), self.tell);
        self.work(
            "handing the work to the yard…",
            move || {
                yardr.bead(&depot, &title, &goal)?;
                match tell {
                    true => yardr.subscribe(&operator, &depot),
                    false => Ok(()),
                }
            },
            |this, made, cx| match made {
                Ok(()) => this.talk(cx),
                Err(why) => this.said = Some(why),
            },
            cx,
        );
    }

    /// The end: the yardmaster's conversation.
    fn talk(&mut self, cx: &mut Context<Self>) {
        cx.emit(Ended::Talking {
            yardmaster: self.form.yardmaster.clone(),
        });
    }

    /// The person leaves the first run where it stands.
    pub fn leave(&mut self, cx: &mut Context<Self>) {
        self.leave_login(cx);
        cx.emit(Ended::Left);
    }

    /// A row of what the yard needs: the program, whether it is there,
    /// and where it is missing how it comes: installed from here where
    /// yardr names a way, and else from its maker's page, with the words
    /// yardr has for it.
    fn row(&self, item: &Item, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let program = item.program.clone();
        let can_fetch = self.means.apple
            && item.fetch.is_some()
            && self.means.machine.home.is_some()
            && self.means.machine.own.is_some();
        let installs = !item.found && (can_fetch || (program == "git" && self.means.apple));
        let state = match (item.found, item.harness.is_some()) {
            (true, _) => "installed",
            (false, true) => "not installed (one harness is enough)",
            (false, false) => "not installed",
        };
        let mut row = div()
            .flex()
            .flex_col()
            .gap_1()
            .debug_selector({
                let program = program.clone();
                move || format!("setup row {program}")
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(program.clone()),
                    )
                    .child(div().text_color(palette.faint).child(state))
                    .when(installs, |el| {
                        let program = program.clone();
                        el.child(
                            self.button(format!("setup install {program}"), "install", palette)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.install(&program, cx)),
                                ),
                        )
                    })
                    .children(item.page().filter(|_| !item.found).map(|page| {
                        let page = page.to_string();
                        self.button(format!("setup page {program}"), "open its page", palette)
                            .on_click(cx.listener(move |this, _, _, cx| this.open(&page, cx)))
                    })),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(palette.faint)
                    .child(item.what_for.clone()),
            );
        // Installed by hand: how, as yardr says it.
        if let Some(get) = item.get.as_ref().filter(|_| !item.found && !installs) {
            let program = program.clone();
            row = row.child(
                div()
                    .debug_selector(move || format!("setup get {program}"))
                    .text_sm()
                    .child(format!("To install it: {get}")),
            );
        }
        row.into_any_element()
    }

    /// A button of the first run, drawn pale while something is underway.
    fn button(&self, id: String, text: &str, palette: Palette) -> gpui::Stateful<Div> {
        small_button(text.to_string(), palette)
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id)
            .when(self.busy.is_some(), |el| el.opacity(0.5))
            // A click on a button leaves the keys where they are: in the
            // field the person was typing in, or is about to.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
    }

    /// A field with its name before it.
    fn typed(
        &self,
        name: &'static str,
        line: &Entity<Line>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focus = line.focus_handle(cx);
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(rems(6.)).flex_none().child(name))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(palette.line)
                    .bg(palette.side)
                    .debug_selector(move || format!("setup {name}"))
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        window.focus(&focus);
                        window.prevent_default();
                    })
                    .child(line.clone()),
            )
            .into_any_element()
    }

    /// What the step shows.
    fn body(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let faint = |text: &str| {
            div()
                .text_sm()
                .text_color(palette.faint)
                .child(text.to_string())
                .into_any_element()
        };
        let plain = |text: &str| div().child(text.to_string()).into_any_element();
        let mut body: Vec<AnyElement> = Vec::new();
        match self.step {
            Step::Yardr => {
                body.push(faint(
                    "footplate uses the yardr on this machine, the program that runs a yard. Where there is none, it can run yardr's own installer from yardr.dev, which puts it in ~/.local/bin. This needs the network once.",
                ));
                if self.busy.is_none() {
                    let label = match self.said.as_deref() {
                        Some(fetch::NOT_THERE) => "install yardr",
                        _ => "try again",
                    };
                    body.push(
                        self.button("setup again".to_string(), label, palette)
                            .on_click(cx.listener(|this, _, _, cx| this.install_yardr(cx)))
                            .into_any_element(),
                    );
                }
            }
            Step::Needs => {
                let Some(doctor) = &self.doctor else {
                    if self.busy.is_none() {
                        body.push(
                            self.button("setup check".to_string(), "check again", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.check(cx)))
                                .into_any_element(),
                        );
                    }
                    return body;
                };
                body.push(faint(
                    "A yard needs git, herdr, which runs each agent, and one harness, the agent program itself.",
                ));
                for item in doctor.rows() {
                    body.push(self.row(item, palette, cx));
                }
                if doctor.server_waits() {
                    body.push(
                        div()
                            .debug_selector(|| "setup server waits".to_string())
                            .text_sm()
                            .text_color(palette.faint)
                            .child("herdr's server is not running yet. That keeps nothing back: starting the yard starts it.")
                            .into_any_element(),
                    );
                }
                let ready = doctor.ready();
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            self.button("setup check".to_string(), "check again", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.check(cx))),
                        )
                        .child(
                            self.button("setup go on".to_string(), "go on", palette)
                                .when(!ready, |el| el.opacity(0.5))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.go_on(window, cx)),
                                ),
                        )
                        .into_any_element(),
                );
                if !ready {
                    body.push(faint(
                        "Going on needs git, herdr and one harness installed. Check again once they are.",
                    ));
                }
            }
            Step::Yard => {
                body.push(self.typed("your name", &self.operator, palette, cx));
                let installed = self.doctor.iter().flat_map(|doctor| doctor.installed());
                let kinds = installed.map(|item| {
                    let kind = item.harness.clone().unwrap_or_default();
                    let text = Self::provider_text(item);
                    let chosen = self.harness.as_deref() == Some(kind.as_str());
                    self.button(format!("setup provider {kind}"), &text, palette)
                        .when(chosen, |el| el.bg(palette.open).text_color(palette.text))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            // Choosing a provider resets the pick to its
                            // recommended entry.
                            this.model_pick = this.recommended(&kind);
                            this.harness = Some(kind.clone());
                            cx.notify();
                        }))
                });
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(div().w(rems(6.)).flex_none().child("provider"))
                        .children(kinds)
                        .into_any_element(),
                );
                body.push(self.typed("yardmaster", &self.yardmaster, palette, cx));
                let offers = self
                    .harness
                    .as_deref()
                    .and_then(|kind| self.installed_item(kind))
                    .map(|item| item.offers.clone())
                    .unwrap_or_default();
                let picks = offers.iter().map(|offer| {
                    let id = offer.model.clone();
                    let text = Self::offer_text(offer);
                    let chosen = self.model_pick.as_deref() == Some(id.as_str());
                    self.button(format!("setup model {id}"), &text, palette)
                        .when(chosen, |el| el.bg(palette.open).text_color(palette.text))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.model_pick = Some(id.clone());
                            cx.notify();
                        }))
                });
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_2()
                        .child(div().w(rems(6.)).flex_none().child("model"))
                        .children(picks)
                        .child(
                            self.button(
                                "setup model own".to_string(),
                                "the harness's own choice",
                                palette,
                            )
                            .when(self.model_pick.is_none(), |el| {
                                el.bg(palette.open).text_color(palette.text)
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model_pick = None;
                                cx.notify();
                            })),
                        )
                        .into_any_element(),
                );
                body.push(self.typed("another model id", &self.model, palette, cx));
                body.push(
                    div()
                        .debug_selector(|| "setup savvy".to_string())
                        .child("How savvy are you with code and a terminal?")
                        .into_any_element(),
                );
                let styles = setup::STYLES.iter().enumerate().map(|(at, (key, _, _))| {
                    let name = setup::style_name(at);
                    self.button(format!("setup style {key}"), &name, palette)
                        .when(at == self.style, |el| {
                            el.bg(palette.open).text_color(palette.text)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.style = at;
                            cx.notify();
                        }))
                });
                body.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(styles)
                        .into_any_element(),
                );
                body.push(
                    div()
                        .debug_selector(|| "setup style note".to_string())
                        .text_sm()
                        .text_color(palette.faint)
                        .child(setup::STYLES[self.style].2.to_string())
                        .into_any_element(),
                );
                body.push(self.typed("about you", &self.about, palette, cx));
                body.push(
                    div()
                        .debug_selector(|| "setup change later".to_string())
                        .text_sm()
                        .text_color(palette.faint)
                        .child("You can change this and the line about you later, in the settings.")
                        .into_any_element(),
                );
                body.push(
                    div()
                        .debug_selector(|| "setup yardmaster note".to_string())
                        .text_sm()
                        .text_color(palette.faint)
                        .child("The yardmaster is the agent you talk with: it should run on the strongest model you have. The override and the line about you may stay empty.")
                        .into_any_element(),
                );
                body.push(
                    div()
                        .flex()
                        .child(
                            self.button("setup create".to_string(), "create (enter)", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                        )
                        .into_any_element(),
                );
            }
            Step::Login => {
                let kind = self.form.harness.clone();
                match (&self.login, self.login_led()) {
                    (None, true) => {
                        body.push(plain(&format!(
                            "The yard is created. Its agents run on {kind}, which wants to be logged in to its maker with your account. footplate keeps nothing of that login."
                        )));
                        body.push(
                            div()
                                .flex()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    self.button("setup log in".to_string(), "log in", palette)
                                        .on_click(cx.listener(|this, _, _, cx| this.log_in(cx))),
                                )
                                .child(
                                    self.button(
                                        "setup logged in".to_string(),
                                        "it is logged in already",
                                        palette,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.skip_login(cx))),
                                )
                                .into_any_element(),
                        );
                    }
                    (None, false) => {
                        body.push(plain(&format!(
                            "The yard is created. Its agents run on {kind}, which needs a terminal once to log in: start {kind} in a terminal and log in on its own screen. That is needed once, and nothing else here needs a terminal."
                        )));
                        body.push(
                            div()
                                .flex()
                                .child(
                                    self.button(
                                        "setup logged in".to_string(),
                                        "it is logged in",
                                        palette,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| this.skip_login(cx))),
                                )
                                .into_any_element(),
                        );
                    }
                    (Some(_), _) => body.extend(self.login_asks(palette, cx)),
                }
            }
            Step::Start => {
                body.push(plain(
                    "Starting the yard starts herdr's server, the yard's own, and the yardmaster.",
                ));
                body.push(
                    div()
                        .flex()
                        .child(
                            self.button("setup start".to_string(), "start the yard", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                        )
                        .into_any_element(),
                );
            }
            Step::Depot => {
                body.push(plain(
                    "The yard runs. Which folder is the work in? A git repository is worked on branches; any other folder is worked in as it is.",
                ));
                body.push(self.typed("folder", &self.folder, palette, cx));
                body.push(
                    div()
                        .flex()
                        .child(
                            self.button("setup choose".to_string(), "choose a folder…", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.choose_folder(cx))),
                        )
                        .into_any_element(),
                );
                body.push(self.typed("name", &self.depot, palette, cx));
                body.push(self.typed("base branch", &self.base, palette, cx));
                body.push(faint(
                    "With no name the folder's is taken, and with no branch the one the folder has checked out. A folder that is no git repository has none.",
                ));
                body.push(self.foot("setup set up", "set up (enter)", Self::set_up, palette, cx));
            }
            Step::Bead => {
                body.push(plain(&format!(
                    "{} is set up. What is the first thing to do there? The builders take it from here, and the yardmaster tells you how it goes.",
                    self.made
                )));
                body.push(self.typed("title", &self.title, palette, cx));
                body.push(self.typed("done when", &self.goal, palette, cx));
                body.push(
                    div()
                        .flex()
                        .child(
                            self.button(
                                "setup tell".to_string(),
                                match self.tell {
                                    true => "☑ tell me when work lands",
                                    false => "☐ tell me when work lands",
                                },
                                palette,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.tell = !this.tell;
                                cx.notify();
                            })),
                        )
                        .into_any_element(),
                );
                body.push(self.foot(
                    "setup hand on",
                    "hand it on (enter)",
                    Self::first_bead,
                    palette,
                    cx,
                ));
            }
        }
        body
    }

    /// The step's own button, with the way past the step beside it: the
    /// yard is whole without a depot, and the yardmaster sets one up in
    /// conversation.
    fn foot(
        &self,
        id: &str,
        text: &str,
        does: fn(&mut Self, &mut Context<Self>),
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .gap_2()
            .child(
                self.button(id.to_string(), text, palette)
                    .on_click(cx.listener(move |this, _, _, cx| does(this, cx))),
            )
            .child(
                self.button("setup later".to_string(), "not now", palette)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.busy.is_none() {
                            this.talk(cx)
                        }
                    })),
            )
            .into_any_element()
    }

    /// What the login that is underway needs: the ways to log in as
    /// buttons, the page as a button, the code as a field, and the
    /// question before the one real call.
    fn login_asks(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut asks: Vec<AnyElement> = Vec::new();
        if let Some(page) = &self.page {
            let page = page.clone();
            asks.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child("Sign in on the harness's page. It may have opened by itself.")
                    .child(
                        div().flex().child(
                            self.button("setup page".to_string(), "open the page", palette)
                                .on_click(cx.listener(move |this, _, _, cx| this.open(&page, cx))),
                        ),
                    )
                    .into_any_element(),
            );
        }
        match &self.need {
            None => asks.push(
                div()
                    .text_color(palette.faint)
                    .child("waiting for the login…")
                    .into_any_element(),
            ),
            Some(Need::Choose { text, options }) => {
                asks.push(div().child(text.clone()).into_any_element());
                let options = options.iter().enumerate().map(|(at, option)| {
                    let chosen = option.clone();
                    self.button(format!("setup method {at}"), option, palette)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let chosen = chosen.clone();
                            this.answer(move |login| login.choose(&chosen), cx)
                        }))
                });
                asks.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .children(options)
                        .into_any_element(),
                );
            }
            Some(Need::Input { .. }) => {
                asks.push(
                    div()
                        .child("If the page gives you a code, put it in here. It is passed on to the harness and kept nowhere.")
                        .into_any_element(),
                );
                asks.push(self.typed("code", &self.code, palette, cx));
                asks.push(
                    div()
                        .flex()
                        .child(
                            self.button("setup code".to_string(), "send (enter)", palette)
                                .on_click(cx.listener(|this, _, _, cx| this.give_code(cx))),
                        )
                        .into_any_element(),
                );
            }
            Some(Need::Confirm { text }) => {
                asks.push(div().child(text.clone()).into_any_element());
                asks.push(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            self.button("setup prove".to_string(), "check it", palette)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.answer(|login| login.confirm(true), cx)
                                })),
                        )
                        .child(
                            self.button("setup no proof".to_string(), "go on without", palette)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.answer(|login| login.confirm(false), cx)
                                })),
                        )
                        .into_any_element(),
                );
            }
            // These end a login, and are not kept as a need.
            Some(Need::Page(_)) | Some(Need::Done { .. }) | Some(Need::Failed(_)) => {}
        }
        asks.push(
            div()
                .flex()
                .child(
                    small_button("stop this login".to_string(), palette)
                        .id("setup stop login")
                        .debug_selector(|| "setup stop login".to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.leave_login(cx);
                            cx.notify();
                        })),
                )
                .into_any_element(),
        );
        asks
    }
}

impl Render for FirstRun {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::now(window, cx);
        div()
            .id("first run")
            .track_focus(&self.focus)
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .items_center()
            .p_8()
            .child(
                // Two divs, not one (reading_column in window.rs stands
                // the same shape): `w_full` with `max_w` keeps the height
                // its text has at full width once the max clamps it,
                // short of the rows it holds.
                div().w_full().child(
                    div()
                        .debug_selector(|| "setup column".to_string())
                        .max_w(rems(36.))
                        .mx_auto()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(palette.faint)
                                                .child("Set up a yard on this machine"),
                                        )
                                        .child(
                                            div()
                                                .debug_selector(|| "setup step".to_string())
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(format!(
                                                    "{} of 5 · {}",
                                                    self.step.number(),
                                                    self.step.title()
                                                )),
                                        ),
                                )
                                .child(
                                    small_button("leave (esc)".to_string(), palette)
                                        .id("setup leave")
                                        .debug_selector(|| "setup leave".to_string())
                                        .on_click(cx.listener(|this, _, _, cx| this.leave(cx))),
                                ),
                        )
                        .children(self.body(palette, cx))
                        .children(self.busy.as_ref().map(|busy| {
                            div()
                                .debug_selector(|| "setup busy".to_string())
                                .text_color(palette.faint)
                                .child(busy.clone())
                        }))
                        .children(self.said.as_ref().map(|said| {
                            div()
                                .debug_selector(|| "setup said".to_string())
                                .child(said.clone())
                        })),
                ),
            )
    }
}

#[cfg(test)]
impl FirstRun {
    /// The provider row as drawn: each installed harness's kind with
    /// its button's text.
    pub fn provider_buttons(&self) -> Vec<(String, String)> {
        self.doctor
            .iter()
            .flat_map(|doctor| doctor.installed())
            .map(|item| {
                (
                    item.harness.clone().unwrap_or_default(),
                    Self::provider_text(item),
                )
            })
            .collect()
    }

    /// The model pick as drawn: each button's picked id (None is the
    /// harness's own choice) with its text.
    pub fn model_buttons(&self) -> Vec<(Option<String>, String)> {
        let mut buttons: Vec<(Option<String>, String)> = self
            .harness
            .as_deref()
            .and_then(|kind| self.installed_item(kind))
            .map(|item| item.offers.clone())
            .unwrap_or_default()
            .iter()
            .map(|offer| (Some(offer.model.clone()), Self::offer_text(offer)))
            .collect();
        buttons.push((None, "the harness's own choice".to_string()));
        buttons
    }

    /// The picked model id: None is the harness's own choice, which a
    /// typed model id still wins over.
    pub fn picked_model(&self) -> Option<String> {
        self.model_pick.clone()
    }

    /// What a test types in the field `name`.
    pub fn type_in(&mut self, name: &str, text: &str, cx: &mut Context<Self>) {
        let field = match name {
            "your name" => &self.operator,
            "yardmaster" => &self.yardmaster,
            "another model id" => &self.model,
            "about you" => &self.about,
            "code" => &self.code,
            "folder" => &self.folder,
            "name" => &self.depot,
            "base branch" => &self.base,
            "title" => &self.title,
            "done when" => &self.goal,
            other => panic!("the first run has no field {other}"),
        };
        field.update(cx, |line, cx| line.set_text(text, cx));
    }

    /// What stands in the field `name`.
    pub fn typed_in(&self, name: &str, cx: &App) -> String {
        let field = match name {
            "your name" => &self.operator,
            "code" => &self.code,
            "name" => &self.depot,
            "base branch" => &self.base,
            other => panic!("the first run has no field {other}"),
        };
        field.read(cx).text().to_string()
    }

    /// The folder is `dir`, as the system's chooser gives one.
    pub fn chose(&mut self, dir: &Path, cx: &mut Context<Self>) {
        self.folder_is(dir, cx);
    }

    /// The PATH of what is started.
    pub fn path(&self) -> Option<std::ffi::OsString> {
        self.yardr.as_ref().map(|yardr| yardr.path.clone())
    }
}
