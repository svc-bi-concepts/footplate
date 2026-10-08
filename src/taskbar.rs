//! The icon and the name a Linux desktop shows for the window, in its
//! taskbar, dock or window list. gpui's window options have no icon, and a
//! Linux desktop takes one from two places, so both are filled when the
//! window opens:
//!
//! - the desktop entry whose id is the window's (`footplate`), with the icon
//!   it names. A Wayland compositor has nothing else to go by, and GNOME and
//!   KDE go by it under X11 too. There is no installer to put it there, so
//!   the program does, under the person's own data directory.
//! - under X11, the window's `_NET_WM_ICON` property, which is all a panel
//!   that reads no desktop entries has.
//!
//! What can be said without a desktop is in plain functions, tested on any
//! system; the rest is for Linux only.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The icon as a file is this size: assets/icon-512.png.
const ICON_SIZE: u32 = 512;

/// Where a person's own desktop entries and icons are: `$XDG_DATA_HOME`
/// when it is an absolute path, `~/.local/share` otherwise (the XDG base
/// directory specification, which has a relative one ignored).
pub fn data_home(xdg_data_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let absolute = |dir: OsString| Some(PathBuf::from(dir)).filter(|dir| dir.is_absolute());
    xdg_data_home
        .and_then(absolute)
        .or_else(|| Some(home.and_then(absolute)?.join(".local/share")))
}

/// The desktop entry of the program at `exe`, or none for a path a desktop
/// entry cannot hold. A desktop finds it by the window's id, which is the
/// file's name and `StartupWMClass`. It is for the window that is open, not
/// for a launcher (`NoDisplay`): the binary is wherever it was built, and an
/// entry in the menu is an installer's to offer.
pub fn entry(name: &str, exe: &Path) -> Option<String> {
    let exec = exec(exe)?;
    Some(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={name}\n\
         Comment=Talk with the crew of a yardr yard\n\
         Exec={exec}\n\
         Icon={name}\n\
         Terminal=false\n\
         Categories=Network;Chat;\n\
         StartupWMClass={name}\n\
         NoDisplay=true\n"
    ))
}

/// A program's path as the `Exec` key takes it: in double quotes, with a
/// backslash before each character that means something within them, those
/// backslashes doubled as in every string of a desktop entry, and `%`, which
/// starts a field code, doubled too.
fn exec(exe: &Path) -> Option<String> {
    let path = exe.to_str()?;
    if path.contains(['\n', '\r']) {
        return None;
    }
    let mut quoted = String::from("\"");
    for letter in path.chars() {
        match letter {
            '"' | '`' | '$' => quoted.push_str("\\\\"),
            '\\' => quoted.push_str("\\\\\\"),
            '%' => quoted.push('%'),
            _ => {}
        }
        quoted.push(letter);
    }
    quoted.push('"');
    Some(quoted)
}

/// Puts the desktop entry and its icon under `data_home`, and says whether
/// anything was written: a file that is there as it should be is left
/// alone, so a start changes nothing after the first. The entry names the
/// binary that ran last.
pub fn install(data_home: &Path, name: &str, exe: &Path, icon: &[u8]) -> io::Result<bool> {
    let entry = entry(name, exe).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("no desktop entry can name {}", exe.display()),
        )
    })?;
    let icon_file = data_home
        .join(format!("icons/hicolor/{ICON_SIZE}x{ICON_SIZE}/apps"))
        .join(format!("{name}.png"));
    let entry_file = data_home
        .join("applications")
        .join(format!("{name}.desktop"));
    // The icon first: a desktop that reads the entry finds what it names.
    let icon_written = put(&icon_file, icon)?;
    let entry_written = put(&entry_file, entry.as_bytes())?;
    Ok(icon_written || entry_written)
}

fn put(file: &Path, content: &[u8]) -> io::Result<bool> {
    if fs::read(file).is_ok_and(|there| there == content) {
        return Ok(false);
    }
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(file, content)?;
    Ok(true)
}

/// One size of the icon as `_NET_WM_ICON` holds it: the width, the height,
/// then a pixel a number, alpha highest and not multiplied in, row by row.
pub fn net_wm_icon(width: u32, height: u32, rgba: &[u8]) -> Vec<u32> {
    let pixels = rgba.as_chunks::<4>().0.iter().map(|&pixel| {
        let [red, green, blue, alpha] = pixel.map(u32::from);
        alpha << 24 | red << 16 | green << 8 | blue
    });
    [width, height].into_iter().chain(pixels).collect()
}

/// Shows `icon` (a PNG) for the window of the application `name`. Nothing
/// here is worth stopping for: what fails is said on stderr, and the window
/// is there without its icon.
#[cfg(target_os = "linux")]
pub fn show(name: &'static str, icon: &'static [u8]) {
    use std::env;

    let installed = data_home(env::var_os("XDG_DATA_HOME"), env::var_os("HOME"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home to put it in"))
        .and_then(|data_home| install(&data_home, name, &env::current_exe()?, icon));
    if let Err(err) = installed {
        eprintln!("footplate: no desktop entry for the taskbar: {err}");
    }
    if gpui::guess_compositor() == "X11" {
        // On a thread: the window is found from outside gpui, once the X
        // server has it, and nothing waits for that.
        std::thread::spawn(move || {
            if let Err(err) = x11::show(name, icon) {
                eprintln!("footplate: no icon on the X11 window: {err}");
            }
        });
    }
}

#[cfg(target_os = "linux")]
mod x11 {
    use std::error::Error;
    use std::thread;
    use std::time::Duration;

    use image::imageops::{self, FilterType};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt, PropMode, Window};
    use x11rb::wrapper::ConnectionExt as _;

    /// What a panel, a window list and a switcher draw, at one and two
    /// device pixels a point.
    const SIZES: [u32; 5] = [32, 48, 64, 128, 256];

    /// The window is on the server soon after gpui opened it: looked for
    /// this often, this far apart, before it is given up.
    const TRIES: u32 = 40;
    const BETWEEN: Duration = Duration::from_millis(250);

    /// Sets `_NET_WM_ICON` on this process's window. gpui gives no handle
    /// to its X11 window (its `window_handle` is `unimplemented!()`), so the
    /// window is found on a connection of our own, by the class gpui sets
    /// from the app id and by the process id.
    pub fn show(name: &str, icon: &[u8]) -> Result<(), Box<dyn Error>> {
        let image = image::load_from_memory_with_format(icon, image::ImageFormat::Png)?.to_rgba8();
        let data: Vec<u32> = SIZES
            .into_iter()
            .flat_map(|size| {
                let scaled = imageops::resize(&image, size, size, FilterType::Lanczos3);
                super::net_wm_icon(size, size, scaled.as_raw())
            })
            .collect();

        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        let pid = connection.intern_atom(false, b"_NET_WM_PID")?.reply()?.atom;
        let net_wm_icon = connection
            .intern_atom(false, b"_NET_WM_ICON")?
            .reply()?
            .atom;
        for _ in 0..TRIES {
            if let Some(window) = find(&connection, root, name, pid)? {
                connection
                    .change_property32(
                        PropMode::REPLACE,
                        window,
                        net_wm_icon,
                        AtomEnum::CARDINAL,
                        &data,
                    )?
                    .check()?;
                return Ok(());
            }
            thread::sleep(BETWEEN);
        }
        Err("the window was not found on the X server".into())
    }

    /// The window of class `name` that this process made, wherever a window
    /// manager has put it under the root. A window that goes away while it
    /// is asked about is no error: it is not ours.
    fn find(
        connection: &impl Connection,
        root: Window,
        name: &str,
        pid: Atom,
    ) -> Result<Option<Window>, Box<dyn Error>> {
        let mut ahead = vec![root];
        while let Some(window) = ahead.pop() {
            let made_by = connection
                .get_property(false, window, pid, AtomEnum::CARDINAL, 0, 1)?
                .reply()
                .ok()
                .and_then(|reply| reply.value32()?.next());
            if made_by == Some(std::process::id()) {
                let class = connection
                    .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)?
                    .reply();
                // The class is two names, each ended by a zero byte.
                if class.is_ok_and(|class| {
                    class
                        .value
                        .split(|&byte| byte == 0)
                        .any(|part| part == name.as_bytes())
                }) {
                    return Ok(Some(window));
                }
            }
            if let Ok(tree) = connection.query_tree(window)?.reply() {
                ahead.extend(tree.children);
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_home_is_the_one_named_else_under_the_home() {
        let dir = |path: &str| Some(OsString::from(path));
        assert_eq!(
            data_home(dir("/data"), dir("/home/ada")),
            Some(PathBuf::from("/data"))
        );
        assert_eq!(
            data_home(None, dir("/home/ada")),
            Some(PathBuf::from("/home/ada/.local/share"))
        );
        // A relative one is ignored, as the specification has it.
        assert_eq!(
            data_home(dir("data"), dir("/home/ada")),
            Some(PathBuf::from("/home/ada/.local/share"))
        );
        assert_eq!(
            data_home(dir(""), dir("/home/ada")),
            Some(PathBuf::from("/home/ada/.local/share"))
        );
        assert_eq!(data_home(None, None), None);
        assert_eq!(data_home(dir("data"), dir("")), None);
    }

    #[test]
    fn the_entry_is_found_by_the_window_s_id_and_names_the_binary_and_the_icon() {
        let entry = entry("footplate", Path::new("/opt/yard/footplate")).unwrap();
        assert_eq!(
            entry,
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=footplate\n\
             Comment=Talk with the crew of a yardr yard\n\
             Exec=\"/opt/yard/footplate\"\n\
             Icon=footplate\n\
             Terminal=false\n\
             Categories=Network;Chat;\n\
             StartupWMClass=footplate\n\
             NoDisplay=true\n"
        );
    }

    #[test]
    fn a_path_is_quoted_as_the_exec_key_takes_it() {
        let exec = |path: &str| exec(Path::new(path));
        assert_eq!(
            exec("/home/ada/my yard/footplate").as_deref(),
            Some(r#""/home/ada/my yard/footplate""#)
        );
        assert_eq!(
            exec(r#"/a"b`c$d\e%f"#).as_deref(),
            Some(r#""/a\\"b\\`c\\$d\\\\e%%f""#)
        );
        // A line break would end the key: no entry.
        assert_eq!(exec("/a\nb"), None);
        assert_eq!(entry("footplate", Path::new("/a\nb")), None);
    }

    #[test]
    fn install_writes_the_entry_and_the_icon_and_then_leaves_them_alone() {
        let dir = tempfile::tempdir().unwrap();
        let exe = Path::new("/opt/yard/footplate");
        let entry_file = dir.path().join("applications/footplate.desktop");
        let icon_file = dir.path().join("icons/hicolor/512x512/apps/footplate.png");

        assert!(install(dir.path(), "footplate", exe, b"png").unwrap());
        assert_eq!(
            fs::read_to_string(&entry_file).unwrap(),
            entry("footplate", exe).unwrap()
        );
        assert_eq!(fs::read(&icon_file).unwrap(), b"png");

        assert!(!install(dir.path(), "footplate", exe, b"png").unwrap());

        // Run from elsewhere, the entry names the binary that ran last.
        let moved = Path::new("/usr/local/bin/footplate");
        assert!(install(dir.path(), "footplate", moved, b"png").unwrap());
        assert!(
            fs::read_to_string(&entry_file)
                .unwrap()
                .contains("Exec=\"/usr/local/bin/footplate\"\n")
        );
    }

    #[test]
    fn install_that_cannot_write_is_an_error_and_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A file where the directory would be.
        fs::write(dir.path().join("icons"), b"").unwrap();
        assert!(install(dir.path(), "footplate", Path::new("/footplate"), b"png").is_err());
        assert!(install(dir.path(), "footplate", Path::new("/a\nb"), b"png").is_err());
    }

    #[test]
    fn the_icon_s_pixels_are_numbers_with_alpha_highest_after_its_size() {
        let rgba = [0x11, 0x22, 0x33, 0xff, 0xaa, 0xbb, 0xcc, 0x80];
        assert_eq!(net_wm_icon(2, 1, &rgba), [2, 1, 0xff11_2233, 0x80aa_bbcc]);
    }

    #[test]
    fn the_icon_the_binary_carries_is_the_size_it_is_installed_as() {
        // The PNG's header: its width and height after the IHDR chunk's name.
        let icon = crate::window::ICON;
        let number = |at: usize| u32::from_be_bytes(icon[at..at + 4].try_into().unwrap());
        assert_eq!(&icon[12..16], b"IHDR");
        assert_eq!((number(16), number(20)), (ICON_SIZE, ICON_SIZE));
    }
}
