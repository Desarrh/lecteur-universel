//! Panneau multimédia de Windows (SystemMediaTransportControls) : titre du fichier dans le menu
//! du volume et sur l'écran de verrouillage, et surtout les touches multimédia du clavier
//! (lecture/pause, suivant, précédent) qui pilotent le lecteur même sans le focus.
//! Repris du module équivalent de Tube.
use windows::Foundation::TypedEventHandler;
use windows::Media::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use windows::core::*;

#[derive(Clone, Copy, Debug)]
pub enum Btn {
    PlayPause,
    Next,
    Previous,
}

/// Ce qui est en cours : identifiant (chemin), titre, vidéo ?, en lecture ?
pub struct Now<'a> {
    pub id: &'a str,
    pub title: &'a str,
    pub video: bool,
    pub playing: bool,
}

pub struct Smtc {
    c: SystemMediaTransportControls,
    shown: String,
    status: i32,
}

impl Smtc {
    pub fn new(hwnd: isize, send: impl Fn(Btn) + Send + Sync + 'static) -> Option<Smtc> {
        let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>().ok()?;
        let c: SystemMediaTransportControls = unsafe { interop.GetForWindow(HWND(hwnd as *mut _)) }.ok()?;
        c.SetIsPlayEnabled(true).ok()?;
        c.SetIsPauseEnabled(true).ok()?;
        c.SetIsNextEnabled(true).ok()?;
        c.SetIsPreviousEnabled(true).ok()?;
        c.ButtonPressed(&TypedEventHandler::new(move |_: Ref<SystemMediaTransportControls>, a: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
            if let Some(a) = a.as_ref() {
                let b = a.Button()?;
                if b == SystemMediaTransportControlsButton::Play || b == SystemMediaTransportControlsButton::Pause {
                    send(Btn::PlayPause);
                } else if b == SystemMediaTransportControlsButton::Next {
                    send(Btn::Next);
                } else if b == SystemMediaTransportControlsButton::Previous {
                    send(Btn::Previous);
                }
            }
            Ok(())
        }))
        .ok()?;
        Some(Smtc { c, shown: String::new(), status: -1 })
    }

    /// `None` : aucun son ni vidéo dans l'onglet actif (panneau masqué).
    pub fn update(&mut self, now: Option<Now>) {
        let Some(now) = now else {
            if self.status != 0 {
                let _ = self.c.SetIsEnabled(false);
                self.status = 0;
                self.shown.clear();
            }
            return;
        };
        if self.shown != now.id {
            self.shown = now.id.to_string();
            let _ = (|| -> Result<()> {
                self.c.SetIsEnabled(true)?;
                let d = self.c.DisplayUpdater()?;
                d.ClearAll()?;
                if now.video {
                    d.SetType(MediaPlaybackType::Video)?;
                    let v = d.VideoProperties()?;
                    v.SetTitle(&HSTRING::from(now.title))?;
                    v.SetSubtitle(&HSTRING::from("Lecteur universel"))?;
                } else {
                    d.SetType(MediaPlaybackType::Music)?;
                    let m = d.MusicProperties()?;
                    m.SetTitle(&HSTRING::from(now.title))?;
                    m.SetArtist(&HSTRING::from("Lecteur universel"))?;
                }
                d.Update()
            })();
            self.status = -1;
        }
        let st = if now.playing { 1 } else { 2 };
        if st != self.status {
            self.status = st;
            let _ = self.c.SetIsEnabled(true);
            let _ = self.c.SetPlaybackStatus(if now.playing { MediaPlaybackStatus::Playing } else { MediaPlaybackStatus::Paused });
        }
    }
}
