use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::Command,
    ptr::{null, null_mut},
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use windows_sys::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM},
        Globalization::GetUserDefaultUILanguage,
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{CreateEventW, INFINITE, SetEvent},
        },
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    w,
};

const CALLBACK: u32 = WM_APP + 1;
const RESTORE_ICON: u32 = WM_APP + 2;
const OPEN_DESKTOP: u32 = WM_APP + 3;
const ICON_ID: u32 = 1;
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

pub(super) struct Tray {
    stop_event: Arc<OwnedHandle>,
    done: oneshot::Receiver<()>,
}

impl Tray {
    pub(super) fn start() -> io::Result<Self> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            return Err(io::Error::last_os_error());
        }
        let stop_event = Arc::new(unsafe { OwnedHandle::from_raw_handle(event) });
        let worker_event = stop_event.clone();
        let (finished, done) = oneshot::channel();
        std::thread::Builder::new()
            .name("hostlane-tray".into())
            .spawn(move || {
                if let Err(err) = run(&worker_event) {
                    tracing::warn!(%err, "Windows notification icon unavailable");
                }
                let _ = finished.send(());
            })?;
        Ok(Self { stop_event, done })
    }

    pub(super) async fn stop(mut self) {
        self.signal();
        let _ = tokio::time::timeout(Duration::from_secs(1), &mut self.done).await;
    }

    fn signal(&self) {
        unsafe {
            SetEvent(self.stop_event.as_raw_handle());
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.signal();
    }
}

fn bundled_desktop(cli: &Path) -> Option<PathBuf> {
    let bin = cli.parent()?;
    let resources = bin.parent()?;
    if !cli.is_absolute() || !bin.ends_with("bin") || !resources.ends_with("resources") {
        return None;
    }
    Some(resources.parent()?.join("Hostlane.exe"))
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg != 0 && TASKBAR_CREATED.get() == Some(&msg) {
            PostMessageW(hwnd, RESTORE_ICON, 0, 0);
        } else if msg == CALLBACK {
            let event = lp as u32 & 0xffff;
            let id = (lp as u32) >> 16;
            if (id == ICON_ID
                && (event == WM_LBUTTONUP || event == NIN_SELECT || event == (NIN_SELECT | 1)))
                || (wp == ICON_ID as usize && lp == WM_LBUTTONUP as isize)
            {
                PostMessageW(hwnd, OPEN_DESKTOP, 0, 0);
            }
        } else if msg != WM_CLOSE {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
    }
    0
}

fn run(stop: &OwnedHandle) -> io::Result<()> {
    let desktop = std::env::current_exe()
        .ok()
        .and_then(|cli| bundled_desktop(&cli))
        .filter(|path| path.is_file());
    unsafe {
        TASKBAR_CREATED.get_or_init(|| RegisterWindowMessageW(w!("TaskbarCreated")));
        let class = WNDCLASSW {
            hInstance: GetModuleHandleW(null()),
            lpszClassName: w!("Hostlane.Daemon.Tray"),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.lpszClassName,
            w!("Hostlane"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            class.hInstance,
            null(),
        );
        let result = if hwnd.is_null() {
            Err(io::Error::last_os_error())
        } else {
            pump(hwnd, stop, desktop.as_deref());
            DestroyWindow(hwnd);
            Ok(())
        };
        UnregisterClassW(class.lpszClassName, class.hInstance);
        result
    }
}

fn pump(hwnd: HWND, stop: &OwnedHandle, desktop: Option<&Path>) {
    #[repr(align(4))]
    struct Icon<const N: usize>([u8; N]);
    static ICON: Icon<{ include_bytes!("../../../docs/icon.png").len() }> =
        Icon(*include_bytes!("../../../docs/icon.png"));
    unsafe {
        let icon = CreateIconFromResourceEx(
            ICON.0.as_ptr(),
            ICON.0.len() as u32,
            1,
            0x30000,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            LR_DEFAULTCOLOR,
        );
        let mut data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
            uCallbackMessage: CALLBACK,
            hIcon: if icon.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                icon
            },
            ..Default::default()
        };
        let tip = match (GetUserDefaultUILanguage() & 0x03ff == 4, desktop.is_some()) {
            (true, true) => "Hostlane 守护进程正在运行 · 点击打开桌面端",
            (true, false) => "Hostlane 守护进程正在运行（独立 CLI）",
            (false, true) => "Hostlane daemon is running · Click to open Hostlane",
            (false, false) => "Hostlane daemon is running (standalone CLI)",
        };
        let mut tooltip = [0u16; 128];
        for (dest, unit) in tooltip.iter_mut().zip(tip.encode_utf16()) {
            *dest = unit;
        }
        data.szTip = tooltip;
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let mut registered = false;
        let mut last_open: Option<Instant> = None;
        'pump: loop {
            if !registered
                && (Shell_NotifyIconW(NIM_ADD, &data) != 0
                    || Shell_NotifyIconW(NIM_MODIFY, &data) != 0)
            {
                Shell_NotifyIconW(NIM_SETVERSION, &data);
                registered = true;
            }
            match MsgWaitForMultipleObjectsEx(
                1,
                &stop.as_raw_handle(),
                if registered { INFINITE } else { 5000 },
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            ) {
                WAIT_TIMEOUT => continue,
                result if result == WAIT_OBJECT_0 + 1 => (),
                _ => break,
            }
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                match msg.message {
                    WM_QUIT => break 'pump,
                    RESTORE_ICON => registered = false,
                    OPEN_DESKTOP => {
                        if let Some(path) = desktop
                            && last_open
                                .is_none_or(|time| time.elapsed() > Duration::from_millis(500))
                        {
                            last_open = Some(Instant::now());
                            if let Err(err) = Command::new(path).spawn() {
                                tracing::warn!(%err, "could not open Hostlane desktop");
                            }
                        }
                    }
                    _ => {
                        DispatchMessageW(&msg);
                    }
                }
            }
        }
        Shell_NotifyIconW(NIM_DELETE, &data);
        if !icon.is_null() {
            DestroyIcon(icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_packaged_cli_has_a_desktop_sibling() {
        assert_eq!(
            bundled_desktop(Path::new(
                r"C:\Program Files\Hostlane\resources\bin\hostlane.exe"
            )),
            Some(PathBuf::from(r"C:\Program Files\Hostlane\Hostlane.exe"))
        );
        for path in [
            r"C:\tools\hostlane.exe",
            r"C:\bin\hostlane.exe",
            r"resources\bin\hostlane.exe",
        ] {
            assert_eq!(bundled_desktop(Path::new(path)), None);
        }
    }

    #[tokio::test]
    async fn notification_window_tracks_daemon_lifetime() {
        let mut tray = Tray::start().unwrap();
        let hwnd = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let mut hwnd = null_mut();
                loop {
                    hwnd = unsafe {
                        FindWindowExW(null_mut(), hwnd, w!("Hostlane.Daemon.Tray"), null())
                    };
                    if hwnd.is_null() {
                        break;
                    }
                    let mut pid = 0;
                    unsafe {
                        GetWindowThreadProcessId(hwnd, &mut pid);
                    }
                    if pid == std::process::id() {
                        return hwnd;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("daemon did not create its notification window");
        assert_eq!(unsafe { IsWindowVisible(hwnd) }, 0);
        let identity = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: ICON_ID,
            ..Default::default()
        };
        if !unsafe { FindWindowW(w!("Shell_TrayWnd"), null()) }.is_null() {
            let wait_for_icon = || async {
                tokio::time::timeout(Duration::from_secs(15), async {
                    while unsafe { Shell_NotifyIconW(NIM_MODIFY, &identity) } == 0 {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                })
                .await
                .expect("Explorer did not register the icon");
            };
            wait_for_icon().await;
            assert_ne!(unsafe { Shell_NotifyIconW(NIM_DELETE, &identity) }, 0);
            unsafe {
                SendMessageW(hwnd, *TASKBAR_CREATED.get().unwrap(), 0, 0);
            }
            wait_for_icon().await;
        }
        tray.signal();
        tokio::time::timeout(Duration::from_secs(10), &mut tray.done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unsafe { IsWindow(hwnd) }, 0);
    }
}
