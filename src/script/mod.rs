// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

mod access;
mod app;
mod composition;
mod host;
mod layout;
mod marker;
mod region;
mod selection;

pub use access::{enter, try_invoke_command};
pub use host::{
    host_from_lua, with_document, EvalOutput, LogEntry, LogLevel, ScriptHost, TestWorld,
    EMBEDDED_INIT,
};

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

    #[test]
    fn define_layout_and_detect_hook() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:define_layout({
              name = "stereo",
              description = "Left / Right",
              channels = { [0] = "L", [1] = "R" },
              monitor = { kind = "passthrough" },
            })
            app:on("detect_layout", function(c)
              if c.channels == 2 then return "stereo" end
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            host.layout("stereo").and_then(|layout| layout.monitor),
            Some(serde_json::json!({ "kind": "passthrough" }))
        );
        let id = world.borrow().active.unwrap();
        host.fire_detect_layout(id);
        let (layout, label0, label1, chosen) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.channel_layout().map(str::to_string),
                composition.channel_label(0),
                composition.channel_label(1),
                composition.chosen_channel_layout().map(str::to_string),
            )
        };
        assert_eq!(layout.as_deref(), Some("stereo"));
        assert_eq!(label0, "L");
        assert_eq!(label1, "R");
        assert!(chosen.is_none());
    }

    #[test]
    fn detect_layout_preserves_chosen() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:define_layout({
              name = "stereo",
              description = "Left / Right",
              channels = { [0] = "L", [1] = "R" },
            })
            app:define_layout({
              name = "MS",
              description = "Mid / Side",
              channels = { [0] = "M", [1] = "S" },
            })
            app:on("detect_layout", function(c, chosen)
              if chosen then return chosen end
              return "stereo"
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let id = world.borrow().active.unwrap();
        host.choose_layout(id, Some("MS")).expect("choose");
        host.fire_detect_layout(id);
        let (chosen, layout, label0) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.chosen_channel_layout().map(str::to_string),
                composition.channel_layout().map(str::to_string),
                composition.channel_label(0),
            )
        };
        assert_eq!(chosen.as_deref(), Some("MS"));
        assert_eq!(layout.as_deref(), Some("MS"));
        assert_eq!(label0, "M");
    }

    #[test]
    fn detect_layout_ignores_unknown_name() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:define_layout({
              name = "stereo",
              description = "Left / Right",
              channels = { [0] = "L", [1] = "R" },
            })
            app:on("detect_layout", function()
              return "not-a-layout"
            end)
            app:on("detect_layout", function()
              return "stereo"
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let id = world.borrow().active.unwrap();
        host.fire_detect_layout(id);
        let prints = host.take_prints();
        assert!(
            prints.iter().any(|line| line.contains("unknown layout")),
            "{prints:?}"
        );
        let layout = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            composition.channel_layout().map(str::to_string)
        };
        assert_eq!(layout.as_deref(), Some("stereo"));
    }

    #[test]
    fn channel_layout_assign_is_user_explicit() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:define_layout({
              name = "MS",
              description = "Mid / Side",
              channels = { [0] = "M", [1] = "S" },
            })
            app.active.channel_layout = "MS"
            return app.active.channel_layout
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("MS"));
        let id = world.borrow().active.unwrap();
        let (chosen, label1) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.chosen_channel_layout().map(str::to_string),
                composition.channel_label(1),
            )
        };
        assert_eq!(chosen.as_deref(), Some("MS"));
        assert_eq!(label1, "S");
    }

    #[test]
    fn composition_exposes_media_metadata() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            local c = app.active
            return c.codec, c.bit_depth, c.basename, c.dirname, c.channels, c.sample_rate
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("pcm\t32\tnil\tnil\t2\t44100"));
    }

    #[test]
    fn embedded_init_defines_default_layouts() {
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        assert_eq!(
            host.layout_names(),
            vec!["mono", "stereo", "MS", "1OA", "2OA"]
        );
        assert_eq!(
            host.layout("stereo").map(|layout| layout.description),
            Some("Left / Right".into())
        );
        let id = world.borrow().active.unwrap();
        host.fire_detect_layout(id);
        let layout = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            composition.channel_layout().map(str::to_string)
        };
        assert_eq!(layout.as_deref(), Some("stereo"));
    }

    #[test]
    fn user_init_takes_precedence_over_embedded() {
        let dir = std::env::temp_dir().join("fieldassist-init-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("init.lua");
        std::fs::write(
            &path,
            r#"
            app:define_layout({
              name = "custom",
              description = "User",
              channels = { [0] = "A", [1] = "B" },
            })
            "#,
        )
        .expect("write init");
        let (mut host, _) = test_host();
        host.load_init_from(Some(&dir)).expect("user init");
        assert_eq!(host.layout_names(), vec!["custom"]);
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn unknown_event_is_an_error() {
        let (mut host, _) = test_host();
        let out = host.eval(r#"app:on("nope", function() end)"#);
        assert!(
            out.error
                .as_deref()
                .is_some_and(|err| err.contains("unknown event")),
            "{:?}",
            out.error
        );
    }

    #[test]
    fn log_methods_are_captured() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app:info("layout", "stereo")
            app:warn("load", "slow")
            app:error("save", "disk full")
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let logs = host.take_logs();
        assert_eq!(logs.len(), 3);
        assert_eq!(logs[0].level, LogLevel::Info);
        assert_eq!(logs[0].topic, "layout");
        assert_eq!(logs[0].message, "stereo");
        assert_eq!(logs[1].level, LogLevel::Warn);
        assert_eq!(logs[1].topic, "load");
        assert_eq!(logs[2].level, LogLevel::Error);
        assert_eq!(logs[2].message, "disk full");
    }

    #[test]
    fn loaded_hook_receives_elapsed() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:on("loaded", function(c, elapsed)
              app:info("load", string.format("%s %.3f", c.name, elapsed))
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let id = world.borrow().active.unwrap();
        host.fire_loaded(id, 0.012);
        let logs = host.take_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].topic, "load");
        assert!(logs[0].message.contains("fixture"), "{:?}", logs[0].message);
        assert!(logs[0].message.contains("0.012"), "{:?}", logs[0].message);
    }

    #[test]
    fn saved_hook_receives_elapsed() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:on("saved", function(c, elapsed)
              app:info("save", string.format("%s %.3f", c.name, elapsed))
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let id = world.borrow().active.unwrap();
        host.fire_saved(id, 0.250);
        let logs = host.take_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].topic, "save");
        assert!(logs[0].message.contains("0.250"), "{:?}", logs[0].message);
    }
}
