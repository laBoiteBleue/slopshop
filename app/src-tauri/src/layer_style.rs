//! Layer > Layer Style > Copy Layer Style and Paste Layer Style (ADR 0032), as Photoshop's: the
//! style copied is kept by the app, not by a document, so that it pastes into any open document
//! (a Pattern Overlay's pattern going with it, as the source it names).

use slopshop_core::LayerId;
use slopshop_core::edit::Edit;
use slopshop_core::session::HistoryLabel;
use tauri::State;

use crate::AppState;
use crate::ipc::DocumentView;

/// Layer `layer_id`'s style copied (its effects, Fill Opacity and whether its effects show), in
/// place of the one copied before. An error when it has none (the menu then offers nothing).
#[tauri::command]
pub async fn copy_layer_style(
    state: State<'_, AppState>,
    document_id: u64,
    layer_id: u64,
) -> Result<(), String> {
    copy(&state, document_id, layer_id)
}

/// The style copied last given to layers `ids` of document `document_id` (adjustment layers
/// aside), one undo entry. An error when none was copied.
#[tauri::command]
pub async fn paste_layer_style(
    state: State<'_, AppState>,
    document_id: u64,
    ids: Vec<u64>,
) -> Result<DocumentView, String> {
    paste(&state, document_id, ids)
}

fn copy(state: &AppState, document_id: u64, layer_id: u64) -> Result<(), String> {
    let style = {
        let mut documents = state.documents()?;
        let document = documents.get_mut(document_id)?;
        let layer = document
            .session
            .document()
            .layer(LayerId::from_raw(layer_id))
            .ok_or("unknown layer")?;
        layer
            .style
            .as_ref()
            .map(|s| s.settings().clone())
            .ok_or("the layer has no style")?
    };
    *state
        .style_clipboard
        .lock()
        .map_err(|_| "the copied style is unavailable")? = Some(style);
    Ok(())
}

fn paste(state: &AppState, document_id: u64, ids: Vec<u64>) -> Result<DocumentView, String> {
    let style = state
        .style_clipboard
        .lock()
        .map_err(|_| "the copied style is unavailable")?
        .clone()
        .ok_or("no layer style was copied")?;
    let mut documents = state.documents()?;
    let document = documents.get_mut(document_id)?;
    let ids: Vec<LayerId> = ids.into_iter().map(LayerId::from_raw).collect();
    let edit =
        Edit::paste_style(document.session.document(), &ids, &style).map_err(|e| e.to_string())?;
    document
        .session
        .with_label(Some(HistoryLabel::new("pasteLayerStyle")), |s| {
            s.perform(edit)
        })
        .map_err(|e| e.to_string())?;
    Ok(document.view())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::style::{LayerStyle, Stroke};
    use slopshop_core::{Session, Size};

    /// A document of three fill layers (ids 1 to 3), opened in `state`: its id.
    fn open(state: &AppState) -> u64 {
        let mut session =
            crate::blank_session(Size::new(8, 8), Some([1.0, 1.0, 1.0]), "Background").unwrap();
        for _ in 0..2 {
            let id = session.allocate_layer_id();
            let layer = slopshop_core::Layer {
                id,
                ..session.document().layers()[0].clone()
            };
            session
                .perform(Edit::InsertLayer {
                    parent: None,
                    index: 0,
                    layer,
                })
                .unwrap();
        }
        state.add_document(session, None, Vec::new()).unwrap().id
    }

    fn style_of(state: &AppState, document: u64, layer: u64) -> Option<LayerStyle> {
        let mut documents = state.documents().unwrap();
        let session: &Session = &documents.get_mut(document).unwrap().session;
        let layer = session.document().layer(LayerId::from_raw(layer)).unwrap();
        layer.style.as_ref().map(|s| s.settings().clone())
    }

    #[test]
    fn a_copied_style_pastes_onto_layers_of_any_document_in_one_undo() {
        let state = AppState::new();
        let (first, second) = (open(&state), open(&state));
        assert!(paste(&state, first, vec![1]).is_err(), "nothing copied yet");
        assert!(copy(&state, first, 1).is_err(), "no style to copy");
        let style = LayerStyle {
            fill_opacity: 0.5,
            stroke: Some(Stroke::default()),
            effects_visible: false,
            ..LayerStyle::default()
        };
        let ids: Vec<u64> = {
            let mut documents = state.documents().unwrap();
            let session = &mut documents.get_mut(first).unwrap().session;
            let ids = session
                .document()
                .layers()
                .iter()
                .map(|l| l.id.get())
                .collect();
            session
                .perform(Edit::SetLayerStyle {
                    id: session.document().layers()[0].id,
                    style: Some(Box::new(style.clone())),
                })
                .unwrap();
            ids
        };
        copy(&state, first, ids[0]).unwrap();
        // Onto two layers of another document, at once (both documents number their layers
        // alike).
        paste(&state, second, vec![ids[1], ids[2]]).unwrap();
        assert_eq!(style_of(&state, second, ids[1]), Some(style.clone()));
        assert_eq!(style_of(&state, second, ids[2]), Some(style));
        assert_eq!(style_of(&state, second, ids[0]), None);
        let mut documents = state.documents().unwrap();
        let session = &mut documents.get_mut(second).unwrap().session;
        session.undo().unwrap();
        assert!(session.document().all_layers().all(|l| l.style.is_none()));
    }
}
