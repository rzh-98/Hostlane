use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use ksni::{MenuItem, Tray, TrayMethods, menu::StandardItem};

pub(super) struct Indicator {
    desktop: Option<PathBuf>,
    chinese: bool,
}

fn desktop_path(cli: &Path, app_dir: Option<&Path>, image: Option<&Path>) -> Option<PathBuf> {
    let root = cli.ancestors().nth(3)?;
    if !cli.is_absolute() || !cli.ends_with("resources/bin/hostlane") {
        return None;
    }
    if let (Some(app_dir), Some(image)) = (app_dir, image)
        && root.parent() == Some(app_dir.join("usr/lib").as_path())
        && app_dir.join("usr/bin/hostlane-desktop").is_file()
        && image.is_absolute()
        && image.is_file()
    {
        return Some(image.to_owned());
    }
    let desktop = root.join("hostlane-desktop");
    if !desktop.is_file() {
        return None;
    }
    if app_dir == Some(root)
        && let Some(image) = image
        && image.is_absolute()
        && image.is_file()
    {
        return Some(image.to_owned());
    }
    Some(desktop)
}

impl Indicator {
    fn open(&self) {
        if let Some(path) = &self.desktop {
            let mut command = tokio::process::Command::new(path);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if let Some(token) = ksni::current_xdg_activation_token() {
                command.env("XDG_ACTIVATION_TOKEN", token);
            }
            tokio::spawn(async move {
                if let Err(err) = command.status().await {
                    tracing::warn!(%err, "could not open Hostlane desktop");
                }
            });
        }
    }
}

impl Tray for Indicator {
    fn id(&self) -> String {
        "hostlane".into()
    }
    fn title(&self) -> String {
        if self.chinese {
            "Hostlane 守护进程正在运行"
        } else {
            "Hostlane daemon is running"
        }
        .into()
    }
    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        const ICON: &[u8; 32 * 32 * 4] = include_bytes!("../assets/tray-32.argb");
        vec![ksni::Icon {
            width: 32,
            height: 32,
            data: ICON.to_vec(),
        }]
    }
    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self.title(),
            ..Default::default()
        }
    }
    fn activate(&mut self, _: i32, _: i32) {
        self.open();
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = vec![
            StandardItem {
                label: self.title(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        if self.desktop.is_some() {
            menu.push(
                StandardItem {
                    label: if self.chinese {
                        "打开 Hostlane"
                    } else {
                        "Open Hostlane"
                    }
                    .into(),
                    activate: Box::new(|tray: &mut Self| tray.open()),
                    ..Default::default()
                }
                .into(),
            );
        }
        menu
    }
}

pub(super) async fn register() -> ksni::Handle<Indicator> {
    let desktop = std::env::current_exe().ok().and_then(|cli| {
        desktop_path(
            &cli,
            std::env::var_os("APPDIR").as_deref().map(Path::new),
            std::env::var_os("APPIMAGE").as_deref().map(Path::new),
        )
    });
    let chinese = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|key| std::env::var(key).ok().filter(|locale| !locale.is_empty()))
        .is_some_and(|locale| locale.starts_with("zh"));
    loop {
        match (Indicator {
            desktop: desktop.clone(),
            chinese,
        })
        .assume_sni_available(true)
        .spawn()
        .await
        {
            Ok(handle) => return handle,
            Err(err) => tracing::debug!(%err, "no tray session bus; will retry"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
