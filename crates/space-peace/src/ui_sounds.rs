//! Button sounds of the command-bar buttons (menu and dialog buttons play their own `GUI_Common_Dialog_*` sounds): `Button_Press` on a press, `GUI_Highlight_Mouse_Over` when the pointer enters
//! a button (event names from `SFXEvents*.xml`).

use alamo_audio::plugin::PlaySfx;
use bevy::prelude::*;

/// Registers the button sound system.
pub struct UiSoundsPlugin;

impl Plugin for UiSoundsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, button_sounds.run_if(resource_exists::<alamo_audio::plugin::AlamoAudio>));
    }
}

fn button_sounds(changed: Query<&Interaction, (Changed<Interaction>, With<eaw_ui::EawButton>)>, mut out: MessageWriter<PlaySfx>) {
    for i in &changed {
        let event = match i {
            Interaction::Pressed => "Button_Press",
            Interaction::Hovered => "GUI_Highlight_Mouse_Over",
            Interaction::None => continue,
        };
        out.write(PlaySfx { event: event.into(), position: None, source: None });
    }
}
