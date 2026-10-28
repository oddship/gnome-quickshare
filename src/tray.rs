use crate::APP_ID;
use crate::i18n::{mark, tr};

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Open,
    Visible(bool),
    Quit,
}
pub struct Tray {
    pub tx: async_channel::Sender<Action>,
}
impl ksni::Tray for Tray {
    const MENU_ON_ACTIVATE: bool = true;
    fn id(&self) -> String {
        APP_ID.into()
    }
    fn title(&self) -> String {
        "Quickshare".into()
    }
    fn icon_name(&self) -> String {
        format!("{APP_ID}-symbolic")
    }
    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        [
            (mark("Open Quickshare"), Action::Open),
            (mark("Visible for 10 minutes"), Action::Visible(true)),
            (mark("Hide this computer"), Action::Visible(false)),
            (mark("Quit"), Action::Quit),
        ]
        .into_iter()
        .map(|(label, action)| {
            ksni::menu::StandardItem {
                label: tr(label),
                activate: Box::new(move |tray: &mut Self| {
                    let _ = tray.tx.try_send(action);
                }),
                ..Default::default()
            }
            .into()
        })
        .collect()
    }
}
