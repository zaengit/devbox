use iced::widget::svg;
use iced::{Color, Element, Length};
use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AppIcon {
    Dashboard,
    Activity,
    Projects,
    Workspaces,
    Runtimes,
    Binary,
    Database,
    Services,
    Globe,
    Extensions,
    Settings,
    Search,
    Refresh,
    Plus,
    Start,
    Stop,
    Restart,
    Check,
    Warning,
    Error,
    Light,
    Dark,
    OpenFolder,
    Scan,
    Logs,
}

impl AppIcon {
    pub const ALL: [Self; 25] = [
        Self::Dashboard,
        Self::Activity,
        Self::Projects,
        Self::Workspaces,
        Self::Runtimes,
        Self::Binary,
        Self::Database,
        Self::Services,
        Self::Globe,
        Self::Extensions,
        Self::Settings,
        Self::Search,
        Self::Refresh,
        Self::Plus,
        Self::Start,
        Self::Stop,
        Self::Restart,
        Self::Check,
        Self::Warning,
        Self::Error,
        Self::Light,
        Self::Dark,
        Self::OpenFolder,
        Self::Scan,
        Self::Logs,
    ];
}

fn bytes(icon: AppIcon) -> &'static [u8] {
    match icon {
        AppIcon::Dashboard => include_bytes!("../../../assets/icons/layout-dashboard.svg"),
        AppIcon::Activity => include_bytes!("../../../assets/icons/activity.svg"),
        AppIcon::Projects => include_bytes!("../../../assets/icons/folder-code.svg"),
        AppIcon::Workspaces => include_bytes!("../../../assets/icons/boxes.svg"),
        AppIcon::Runtimes => include_bytes!("../../../assets/icons/terminal.svg"),
        AppIcon::Binary => include_bytes!("../../../assets/icons/binary.svg"),
        AppIcon::Database => include_bytes!("../../../assets/icons/database.svg"),
        AppIcon::Services => include_bytes!("../../../assets/icons/server.svg"),
        AppIcon::Globe => include_bytes!("../../../assets/icons/globe.svg"),
        AppIcon::Extensions => include_bytes!("../../../assets/icons/puzzle.svg"),
        AppIcon::Settings => include_bytes!("../../../assets/icons/settings.svg"),
        AppIcon::Search => include_bytes!("../../../assets/icons/search.svg"),
        AppIcon::Refresh => include_bytes!("../../../assets/icons/refresh-cw.svg"),
        AppIcon::Plus => include_bytes!("../../../assets/icons/plus.svg"),
        AppIcon::Start => include_bytes!("../../../assets/icons/play.svg"),
        AppIcon::Stop => include_bytes!("../../../assets/icons/square.svg"),
        AppIcon::Restart => include_bytes!("../../../assets/icons/rotate-ccw.svg"),
        AppIcon::Check => include_bytes!("../../../assets/icons/check.svg"),
        AppIcon::Warning => include_bytes!("../../../assets/icons/triangle-alert.svg"),
        AppIcon::Error => include_bytes!("../../../assets/icons/circle-x.svg"),
        AppIcon::Light => include_bytes!("../../../assets/icons/sun.svg"),
        AppIcon::Dark => include_bytes!("../../../assets/icons/moon.svg"),
        AppIcon::OpenFolder => include_bytes!("../../../assets/icons/folder-open.svg"),
        AppIcon::Scan => include_bytes!("../../../assets/icons/scan-search.svg"),
        AppIcon::Logs => include_bytes!("../../../assets/icons/square-terminal.svg"),
    }
}

static HANDLES: LazyLock<HashMap<AppIcon, svg::Handle>> = LazyLock::new(|| {
    AppIcon::ALL
        .into_iter()
        .map(|icon| (icon, svg::Handle::from_memory(bytes(icon))))
        .collect()
});

pub fn icon<'a, Message: 'a>(icon: AppIcon, size: f32, color: Color) -> Element<'a, Message> {
    svg(HANDLES
        .get(&icon)
        .expect("all icons are registered")
        .clone())
    .width(Length::Fixed(size))
    .height(Length::Fixed(size))
    .style(move |_, _| svg::Style { color: Some(color) })
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_icon_maps_to_a_distinct_bundled_svg() {
        assert_eq!(HANDLES.len(), AppIcon::ALL.len());
        let mut assets = HashSet::new();
        for icon in AppIcon::ALL {
            let bytes = bytes(icon);
            let source = std::str::from_utf8(bytes).expect("bundled SVG is UTF-8");
            assert!(source.contains("<svg"), "{icon:?} must map to an SVG asset");
            assert!(
                assets.insert(bytes),
                "{icon:?} must not reuse another asset"
            );
            let _: Element<'_, ()> = self::icon(icon, 20.0, Color::from_rgb(0.0, 0.0, 0.0));
        }
    }
}
