// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

mod access;
mod app;
mod composition;
mod host;
mod marker;
mod region;
mod selection;

pub use access::{enter, try_invoke_command};
pub use host::{host_from_lua, with_document, EvalOutput, ScriptHost, TestWorld};

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use crate::model::composition::{Composition, MediaId, MediaRef};
    use crate::model::Buffer;

    use super::*;

    fn test_host() -> (ScriptHost, Rc<RefCell<TestWorld>>) {
        let world = Rc::new(RefCell::new(TestWorld::new()));
        let samples = vec![vec![0.0; 1000], vec![0.0; 1000]];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        let composition = Composition::from_media(media).expect("composition");
        world
            .borrow_mut()
            .push(composition, Buffer::empty(), "fixture", None);
        let host = ScriptHost::for_test(world.clone()).expect("lua");
        (host, world)
    }

    #[test]
    fn eval_returns_expression_results() {
        let (mut host, _) = test_host();
        let out = host.eval("1 + 2");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("3"));
    }

    #[test]
    fn print_is_captured() {
        let (mut host, _) = test_host();
        let out = host.eval(r#"print("hello")"#);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.prints, vec!["hello".to_string()]);
    }

    #[test]
    fn unknown_command_is_an_error() {
        let (mut host, _) = test_host();
        let out = host.eval(r#"app:command("not.a.command")"#);
        assert!(
            out.error
                .as_deref()
                .is_some_and(|err| err.contains("unknown command")),
            "{:?}",
            out.error
        );
    }

    #[test]
    fn selection_and_named_region_round_trip() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local c = app.active
            c:select(0, 100)
            local region = c:add_region({
              start = 10,
              stop = 20,
              label = "intro",
              collection = "cues",
            })
            local sel = c.selection.regions[1]
            return sel.start, sel.stop, region.label, region.start, region.collection, #c.collections
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("0\t100\tintro\t10\tcues\t2"));
        let world = world.borrow();
        let id = world.active.unwrap();
        let doc = world.docs.get(&id).unwrap();
        let composition = doc.composition.read().unwrap();
        let cues = composition.collection("cues").expect("cues");
        assert_eq!(cues.regions[0].label.as_deref(), Some("intro"));
    }

    #[test]
    fn collections_and_marker_types_round_trip() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local c = app.active
            c:clear_selection()
            c:add_region({ start = 5, stop = 15, label = "sel" })
            local silent = c:collection("silent")
            c:add_region({ start = 40, stop = 50, collection = "silent", label = "gap" })
            c:add_marker_type("Red", {1, 0, 0, 1})
            c:add_marker({ frame = 12, type = "Red", note = "hit" })
            local types_before, markers_before = #c.marker_types, #c.markers
            c:remove_marker_type("Red")
            return c.selection.regions[1].label, silent.regions[1].label, types_before, markers_before, #c.marker_types, #c.markers
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("sel\tgap\t4\t1\t3\t0"));
        let world = world.borrow();
        let id = world.active.unwrap();
        let doc = world.docs.get(&id).unwrap();
        assert_eq!(doc.selection.regions[0].label.as_deref(), Some("sel"));
        assert!(doc
            .composition
            .read()
            .unwrap()
            .marker_types()
            .iter()
            .all(|ty| ty.name != "Red"));
    }

    #[test]
    fn markers_can_be_created_listed_and_removed() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local c = app.active
            local a = c:add_marker({ frame = 40, type = "Blue", note = "cue" })
            local b = c:add_marker(40, "Yellow")
            local dup = c:add_marker(40, "Blue")
            local at_blue = c:marker_at(40, "Blue")
            local frame, kind, note = at_blue.frame, at_blue.type, at_blue.note
            c:remove_marker(a)
            b:remove()
            c:add_marker(80)
            c:remove_marker_at(80)
            return dup == nil, frame, kind, note, #c.markers
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true\t40\tBlue\tcue\t0"));
        let world = world.borrow();
        let id = world.active.unwrap();
        let doc = world.docs.get(&id).unwrap();
        assert!(doc.composition.read().unwrap().markers().is_empty());
    }

    #[test]
    fn add_marker_color_registers_unknown_type() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local c = app.active
            local m = c:add_marker({ frame = 10, type = "Red", color = {1, 0, 0, 1} })
            return m.type, m.color[1], m.color[2], m.color[3], #c.marker_types
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("Red\t1.0\t0.0\t0.0\t4"));
        let world = world.borrow();
        let id = world.active.unwrap();
        let doc = world.docs.get(&id).unwrap();
        let composition = doc.composition.read().unwrap();
        assert!(composition
            .marker_types()
            .iter()
            .any(|ty| ty.name == "Red" && ty.color == [1.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn known_command_id_is_accepted() {
        let (mut host, _) = test_host();
        let out = host.eval(r#"app:command("edit.trim")"#);
        assert!(out.error.is_none(), "{:?}", out.error);
    }
}
