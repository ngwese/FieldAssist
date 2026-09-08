// SPDX-FileCopyrightText: 2026 Greg Wuller
// SPDX-License-Identifier: MIT

mod access;
mod app;
mod composition;
mod files;
mod host;
mod layout;
mod marker;
mod region;
mod selection;
mod session;
mod theme;
mod workflow;

pub use access::{enter, try_invoke_command};
pub use host::{
    host_from_lua, with_document, EvalOutput, LogEntry, LogLevel, ResumeWorkflow, ScriptHost,
    TestWorld, EMBEDDED_INIT,
};
pub use workflow::{DropLayout, PathBrowse, ToolbarAlign, ToolbarItem};

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;
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

    fn item_kind(item: &ToolbarItem) -> &'static str {
        match item {
            ToolbarItem::Button { .. } => "button",
            ToolbarItem::Path { .. } => "path",
            ToolbarItem::Toggle { .. } => "toggle",
            ToolbarItem::Message { .. } => "message",
            ToolbarItem::Divider { .. } => "divider",
        }
    }

    fn message_text(item: &ToolbarItem) -> Option<&str> {
        match item {
            ToolbarItem::Message { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }

    fn toggle_value(item: &ToolbarItem) -> Option<bool> {
        match item {
            ToolbarItem::Toggle { value, .. } => Some(*value),
            _ => None,
        }
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
    fn monitor_chain_and_playback_channels_round_trip() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:define_layout({
              name = "MS",
              description = "Mid / Side",
              channels = { [0] = "M", [1] = "S" },
              monitor = { chain = "ms" },
            })
            local c = app.active
            c.channel_layout = "MS"
            assert(c.monitor_chain == "ms")
            c.monitor_chain = "stereo"
            c.playback_channels = {1}
            return c.monitor_chain, c.playback_channels[1]
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("stereo\t1"));
        let id = world.borrow().active.unwrap();
        let (chain, channels) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.monitor_chain().map(str::to_string),
                composition.playback_channels().map(|ch| ch.to_vec()),
            )
        };
        assert_eq!(chain.as_deref(), Some("stereo"));
        assert_eq!(channels.as_deref(), Some(&[1][..]));
        let out = host.eval(
            r#"
            app.active.playback_channels = "all"
            app.active.monitor_chain = nil
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let (chain, channels) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.monitor_chain().map(str::to_string),
                composition.playback_channels().map(|ch| ch.to_vec()),
            )
        };
        assert!(chain.is_none());
        assert!(channels.is_none());
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
            vec![
                "mono",
                "stereo",
                "MS",
                "B-Format (AmbiX)",
                "B-Format (FuMa)",
                "2OA"
            ]
        );
        assert_eq!(
            host.layout("stereo").map(|layout| layout.description),
            Some("Left / Right".into())
        );
        let id = world.borrow().active.unwrap();
        host.fire_detect_layout(id);
        let (layout, chain) = {
            let world = world.borrow();
            let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
            (
                composition.channel_layout().map(str::to_string),
                composition.monitor_chain().map(str::to_string),
            )
        };
        assert_eq!(layout.as_deref(), Some("stereo"));
        assert_eq!(chain.as_deref(), Some("stereo"));
        assert_eq!(
            host.layout("B-Format (AmbiX)")
                .and_then(|layout| layout.monitor_chain_id().map(str::to_string))
                .as_deref(),
            Some("foa")
        );
        assert_eq!(
            host.layout("B-Format (FuMa)")
                .and_then(|layout| layout.monitor_chain_id().map(str::to_string))
                .as_deref(),
            Some("foa_fuma")
        );
        assert!(host.layout("1OA").is_none());
        assert!(host.layout("2OA").unwrap().monitor_chain_id().is_none());
    }

    fn detect_with(channels: usize, filename: Option<&str>) -> (Option<String>, Option<String>) {
        let world = Rc::new(RefCell::new(TestWorld::new()));
        let samples = vec![vec![0.0f32; 64]; channels];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        let composition = Composition::from_media(media).expect("composition");
        let path = filename.map(std::path::PathBuf::from);
        let id = world
            .borrow_mut()
            .push(composition, Buffer::empty(), "fixture", path);
        let mut host = ScriptHost::for_test(world.clone()).expect("lua");
        host.load_init_from(None).expect("embedded init");
        host.fire_detect_layout(id);
        let world = world.borrow();
        let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
        (
            composition.channel_layout().map(str::to_string),
            composition.monitor_chain().map(str::to_string),
        )
    }

    #[test]
    fn detect_layout_uses_basename_keywords_and_count() {
        let (layout, chain) = detect_with(4, Some("Take_Ambix.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (AmbiX)"));
        assert_eq!(chain.as_deref(), Some("foa"));

        let (layout, chain) = detect_with(4, Some("Take_FuMa.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (FuMa)"));
        assert_eq!(chain.as_deref(), Some("foa_fuma"));

        let (layout, _) = detect_with(4, Some("session_ambix_bformat.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (AmbiX)"));

        let (layout, chain) = detect_with(4, None);
        assert_eq!(layout.as_deref(), Some("B-Format (AmbiX)"));
        assert_eq!(chain.as_deref(), Some("foa"));

        let (layout, chain) = detect_with(6, Some("Ambix_plus_xy.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (AmbiX)"));
        assert_eq!(chain.as_deref(), Some("foa"));

        let (layout, chain) = detect_with(6, Some("FuMa_plus_xy.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (FuMa)"));
        assert_eq!(chain.as_deref(), Some("foa_fuma"));

        let (layout, _) = detect_with(6, Some("Ambix_and_FuMa.wav"));
        assert_eq!(layout.as_deref(), Some("B-Format (FuMa)"));
    }

    #[test]
    fn detect_layout_remaps_persisted_1oa() {
        let world = Rc::new(RefCell::new(TestWorld::new()));
        let samples = vec![vec![0.0f32; 64]; 4];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        let composition = Composition::from_media(media).expect("composition");
        let id = world
            .borrow_mut()
            .push(composition, Buffer::empty(), "fixture", None);
        {
            let world = world.borrow();
            world
                .docs
                .get(&id)
                .unwrap()
                .composition
                .write()
                .unwrap()
                .choose_channel_layout(Some("1OA".into()), Default::default());
        }
        let mut host = ScriptHost::for_test(world.clone()).expect("lua");
        host.load_init_from(None).expect("embedded init");
        host.fire_detect_layout(id);
        let world = world.borrow();
        let composition = world.docs.get(&id).unwrap().composition.read().unwrap();
        assert_eq!(composition.chosen_channel_layout(), Some("1OA"));
        assert_eq!(composition.channel_layout(), Some("B-Format (AmbiX)"));
        assert_eq!(composition.monitor_chain(), Some("foa"));
        assert_eq!(composition.channel_label(1), "Y");
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
    fn theme_named_and_semantic_colors_are_rgba_tables() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            local function check(color)
              assert(type(color) == "table")
              assert(#color == 4)
              for i = 1, 4 do
                assert(type(color[i]) == "number")
                assert(color[i] >= 0 and color[i] <= 1)
              end
            end
            check(app.theme.named.green)
            check(app.theme.named.red_light)
            check(app.theme.semantic.success)
            check(app.theme.semantic.warning)
            check(app.theme.semantic.info)
            app:declare_workflow({
              name = "themed",
              scopes = { "drag-drop" },
              drop = { color = app.theme.named.green },
            }, function() end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let unknown = host.eval("return app.theme.named.nope");
        assert!(
            unknown
                .error
                .as_deref()
                .is_some_and(|err| err.contains("unknown field")),
            "{:?}",
            unknown.error
        );
        let metas = host.workflow_metas();
        let themed = metas
            .iter()
            .find(|meta| meta.name == "themed")
            .expect("themed");
        let expected = {
            let rgba: gpui::Rgba = gpui_component::ThemeColor::default().green.into();
            [rgba.r, rgba.g, rgba.b, rgba.a]
        };
        assert_eq!(themed.color, expected);
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

    #[test]
    fn output_device_round_trips_in_test_world() {
        let (mut host, world) = test_host();

        let out = host.eval("return #app.output_devices");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("0"));

        world.borrow_mut().output_devices = vec!["Speakers (Realtek)".into(), "Headphones".into()];

        let out = host.eval("return app.output_device == nil");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true"));

        let out = host.eval(
            r#"
            app.output_device = "Speakers (Realtek)"
            return app.output_device
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("Speakers (Realtek)"));
        assert_eq!(
            world.borrow().output_device.as_deref(),
            Some("Speakers (Realtek)")
        );

        let out = host.eval(
            r#"
            app.output_device = nil
            return app.output_device == nil
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true"));
        assert!(world.borrow().output_device.is_none());

        let out = host.eval(
            r#"
            return #app.output_devices, app.output_devices[1], app.output_devices[2]
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            out.result.as_deref(),
            Some("2\tSpeakers (Realtek)\tHeadphones")
        );
    }

    #[test]
    fn output_device_rejects_non_string() {
        let (mut host, _) = test_host();
        let out = host.eval("app.output_device = 1");
        assert!(
            out.error
                .as_deref()
                .is_some_and(|err| err.contains("string or nil")),
            "{:?}",
            out.error
        );
    }

    #[test]
    fn session_and_document_metadata_round_trip() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local s = app.session
            s.workflow = "review"
            s.capture_ui = false
            s.properties = { batch = "2026-09" }
            local c = app.active
            c.group = "day1"
            c.state = "reviewed"
            c.properties = { reviewer = "greg" }
            return s.id ~= nil, s.workflow, s.capture_ui, s.properties.batch,
                   c.id, c.group, c.state, c.properties.reviewer, #app.documents, #s.documents
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let result = out.result.expect("result");
        let parts: Vec<&str> = result.split('\t').collect();
        assert_eq!(parts[0], "true");
        assert_eq!(parts[1], "review");
        assert_eq!(parts[2], "false");
        assert_eq!(parts[3], "2026-09");
        assert_eq!(parts[5], "day1");
        assert_eq!(parts[6], "reviewed");
        assert_eq!(parts[7], "greg");
        assert_eq!(parts[8], "1");
        assert_eq!(parts[9], "1");
        {
            let world = world.borrow();
            assert_eq!(world.session.workflow(), Some("review"));
            assert!(!world.session.capture_ui());
            assert_eq!(
                world.session.properties().get("batch").map(String::as_str),
                Some("2026-09")
            );
            let id = world.active.unwrap();
            let doc = world.session.get(id).unwrap();
            assert_eq!(doc.group.as_deref(), Some("day1"));
            assert_eq!(doc.state.as_deref(), Some("reviewed"));
            assert_eq!(
                doc.properties.get("reviewer").map(String::as_str),
                Some("greg")
            );
        }

        let out = host.eval(
            r#"
            app.session.workflow = nil
            app.active.group = nil
            app.active.state = nil
            app.active.properties = {}
            return app.session.workflow == nil, app.active.group == nil,
                   app.active.state == nil, app.active.properties.reviewer == nil
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true\ttrue\ttrue\ttrue"));
    }

    #[test]
    fn session_group_count_matches_assigned_groups() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app.active.group = "todo"
            return app.session:group_count("todo"), app.session:group_count("other")
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("1\t0"));
    }

    #[test]
    fn session_properties_reject_non_strings() {
        let (mut host, _) = test_host();
        let out = host.eval(r#"app.session.properties = { batch = 1 }"#);
        assert!(
            out.error
                .as_deref()
                .is_some_and(|err| err.contains("must be a string")),
            "{:?}",
            out.error
        );
    }

    #[test]
    fn composition_close_removes_from_session() {
        let (mut host, world) = test_host();
        let out = host.eval("app.active:close(); return app.active == nil, #app.documents");
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true\t0"));
        assert!(world.borrow().session.is_empty());
    }

    #[test]
    fn session_save_as_writes_json() {
        let (mut host, world) = test_host();
        let dir = std::env::temp_dir().join("fa-lua-session");
        let _ = std::fs::create_dir_all(&dir);
        let wav = dir.join("take.wav");
        std::fs::write(&wav, b"wav").unwrap();
        {
            let mut world = world.borrow_mut();
            let id = world.active.unwrap();
            world.session.set_project_path(id, wav.clone());
        }
        let path = dir.join("batch.fasession");
        let path_lua = path.to_string_lossy().replace('\\', "/");
        let out = host.eval(&format!(
            r#"
            app.session.workflow = "review"
            app.session:save_as("{path_lua}")
            return app.session.path ~= nil
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.result.as_deref(), Some("true"));
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("fasession"), "{json}");
        assert!(json.contains("review"), "{json}");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_loaded_hook_runs() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app:on("session_loaded", function(s)
              app:info("session", s.id)
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.fire_session_loaded();
        let logs = host.take_logs();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].topic, "session");
        assert!(!logs[0].message.is_empty());
    }

    #[test]
    fn declare_workflow_replaces_by_name_and_defaults() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app:declare_workflow({
              name = "add",
              scopes = { "drag-drop" },
            }, function() end)
            app:declare_workflow({
              name = "add",
              display_name = "Merge",
              scopes = { "drag-drop" },
              drop = { row = 2, priority = 3 },
            }, function() end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let metas = host.workflow_metas();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "add");
        assert_eq!(metas[0].display_name, "Merge");
        assert_eq!(metas[0].row, 2);
        assert_eq!(metas[0].priority, 3.0);
    }

    #[test]
    fn embedded_workflows_register_add_replace_and_review() {
        let (mut host, _) = test_host();
        host.load_init_from(None).expect("embedded init");
        let mut names: Vec<_> = host
            .workflow_metas()
            .into_iter()
            .map(|meta| meta.name)
            .collect();
        names.sort();
        assert_eq!(names, ["add", "replace", "review"]);
        let layout = host.drop_layout();
        assert_eq!(layout.rows.len(), 2);
        let display: Vec<_> = layout.rows[0]
            .cells
            .iter()
            .map(|cell| cell.display_name.as_str())
            .collect();
        assert_eq!(display, ["Add", "Replace"]);
        assert_eq!(layout.rows[0].cells[0].weight, 0.5);
        assert_eq!(layout.rows[0].cells[1].weight, 0.5);
        assert_eq!(layout.rows[1].cells.len(), 1);
        assert_eq!(layout.rows[1].cells[0].display_name, "Review");
        assert_eq!(layout.rows[1].cells[0].weight, 1.0);
        assert_eq!(
            host.menu_workflows(),
            vec![("review".into(), "Review".into())]
        );
    }

    #[test]
    fn review_workflow_logs_dropped_paths() {
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        let _ = host.take_logs();
        host.invoke_workflow(
            "review",
            &[PathBuf::from("take.wav"), PathBuf::from("batch.fasession")],
        )
        .expect("review");
        let logs = host.take_logs();
        assert!(logs
            .iter()
            .all(|entry| entry.level == LogLevel::Info && entry.topic == "review"));
        let messages: Vec<_> = logs.iter().map(|entry| entry.message.as_str()).collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("2 path") && message.contains("drag-drop")),
            "{messages:?}"
        );
        assert!(messages.iter().any(|message| message.contains("take.wav")));
        assert!(messages
            .iter()
            .any(|message| message.contains("batch.fasession")));
        assert_eq!(world.borrow().session.workflow(), Some("review"));
        assert!(world.borrow().looping);
        assert!(world.borrow().preview);
        assert!(world.borrow().explorer);
        let snapshot = host.toolbar_snapshot().expect("toolbar");
        assert_eq!(snapshot.0, "Review");
        let kinds: Vec<_> = snapshot.1.iter().map(item_kind).collect();
        assert_eq!(
            kinds,
            ["button", "button", "button", "toggle", "divider", "message", "path", "button"]
        );
        assert_eq!(snapshot.1[0].command(), Some("previous"));
        assert_eq!(snapshot.1[1].command(), Some("next"));
        assert_eq!(snapshot.1[2].command(), Some("drop"));
        assert_eq!(snapshot.1[0].align(), crate::script::ToolbarAlign::Left);
        assert_eq!(snapshot.1[6].align(), crate::script::ToolbarAlign::Right);
        assert_eq!(snapshot.1[7].command(), Some("finish"));
        assert_eq!(snapshot.1[7].align(), crate::script::ToolbarAlign::Right);
        host.dispatch_workflow_command("next").expect("next");
        host.finish_workflow().expect("finish");
        assert!(world.borrow().session.workflow().is_none());
        assert!(host.toolbar_snapshot().is_none());
        assert!(!world.borrow().looping);
        assert!(!world.borrow().preview);
        assert!(world.borrow().explorer);
    }

    #[test]
    fn find_files_filters_by_extension_list_and_predicate() {
        let dir = std::env::temp_dir().join("fieldassist-lua-find-files");
        let nested = dir.join("nested");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&nested).expect("dir");
        std::fs::write(dir.join("take.wav"), b"wav").expect("wav");
        std::fs::write(nested.join("more.flac"), b"flac").expect("flac");
        std::fs::write(nested.join("notes.txt"), b"txt").expect("txt");
        std::fs::write(dir.join("edit.facomp"), b"{}").expect("facomp");
        let path_lua = dir.to_string_lossy().replace('\\', "/");
        let (mut host, _) = test_host();
        let out = host.eval(&format!(
            r#"
            local dir = "{path_lua}"
            local by_ext = app:find_files(dir, {{ "wav", ".FLAC", "facomp" }})
            local by_fn = app:find_files(dir, function(dirname, basename)
              return basename:match("%.txt$") ~= nil and dirname == "nested"
            end)
            return table.concat(by_ext, ","), table.concat(by_fn, ","), #app:find_files(dir)
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            out.result.as_deref(),
            Some("edit.facomp,nested/more.flac,take.wav\tnested/notes.txt\t4")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn review_expands_folders_and_groups_documents_todo() {
        let dir = std::env::temp_dir().join("fieldassist-review-folder");
        let nested = dir.join("nested");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&nested).expect("dir");
        let wav = dir.join("take.wav");
        let flac = nested.join("more.flac");
        std::fs::write(&wav, b"wav").expect("wav");
        std::fs::write(&flac, b"flac").expect("flac");
        std::fs::write(nested.join("notes.txt"), b"txt").expect("txt");
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        let _ = host.take_logs();
        host.invoke_workflow("review", &[dir.clone()])
            .expect("review");
        let groups: Vec<_> = world
            .borrow()
            .session
            .documents()
            .iter()
            .filter_map(|doc| {
                let path = doc.file_path()?;
                if path == wav.as_path() || path == flac.as_path() {
                    Some((path.to_path_buf(), doc.group.clone()))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(groups.len(), 2, "{groups:?}");
        assert!(groups
            .iter()
            .all(|(_, group)| group.as_deref() == Some("todo")));
        assert!(world
            .borrow()
            .session
            .documents()
            .iter()
            .all(|doc| doc.file_path() != Some(nested.join("notes.txt").as_path())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn menu_workflows_sorted_alphabetically_by_display_name() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app:declare_workflow({
              name = "zeta",
              display_name = "Zebra",
              scopes = { "menu" },
            }, function() end)
            app:declare_workflow({
              name = "alpha",
              display_name = "Apple",
              scopes = { "drag-drop", "menu" },
            }, function() end)
            app:declare_workflow({
              name = "drop",
              display_name = "Drop Only",
              scopes = { "drag-drop" },
            }, function() end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        let menu: Vec<_> = host
            .menu_workflows()
            .into_iter()
            .map(|(_, display)| display)
            .collect();
        assert_eq!(menu, ["Apple", "Zebra"]);
    }

    #[test]
    fn menu_start_omits_paths_and_uses_menu_scope() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            app:declare_workflow({
              name = "probe",
              display_name = "Probe",
              scopes = { "menu" },
            }, function(payload)
              app:info("probe", payload.scope)
              app:info("probe", tostring(payload.paths))
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_menu_workflow("probe").expect("menu start");
        let messages: Vec<_> = host
            .take_logs()
            .into_iter()
            .map(|entry| entry.message)
            .collect();
        assert_eq!(messages, ["menu", "nil"]);
    }

    #[test]
    fn review_menu_start_marks_open_documents_todo() {
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        let _ = host.take_logs();
        assert!(world
            .borrow()
            .session
            .documents()
            .iter()
            .all(|doc| doc.group.is_none()));
        host.invoke_menu_workflow("review").expect("review menu");
        let logs = host.take_logs();
        assert!(
            logs.iter()
                .any(|entry| entry.message.contains("menu") && entry.message.contains("todo")),
            "{logs:?}"
        );
        assert!(world
            .borrow()
            .session
            .documents()
            .iter()
            .all(|doc| doc.group.as_deref() == Some("todo")));
        assert_eq!(world.borrow().session.workflow(), Some("review"));
        assert_eq!(host.active_workflow_name().as_deref(), Some("review"));
        assert!(world.borrow().looping);
        assert!(world.borrow().preview);
        assert!(world.borrow().explorer);
        let snapshot = host.toolbar_snapshot().expect("toolbar");
        assert_eq!(message_text(&snapshot.1[5]), Some("0 of 1 files reviewed"));
    }

    #[test]
    fn review_next_cycles_todo_and_reviewed_updates_group() {
        let (mut host, world) = test_host();
        let samples = vec![vec![0.0; 1000], vec![0.0; 1000]];
        let media = MediaRef::from_memory(MediaId(0), 44100, samples);
        let composition = Composition::from_media(media).expect("composition");
        let second = world
            .borrow_mut()
            .push(composition, Buffer::empty(), "second", None);
        host.load_init_from(None).expect("embedded init");
        let _ = host.take_logs();
        host.invoke_menu_workflow("review").expect("review menu");
        let first = world
            .borrow()
            .session
            .documents()
            .first()
            .map(|doc| doc.id)
            .expect("first");
        assert_eq!(world.borrow().active, Some(second));
        host.dispatch_workflow_command("previous")
            .expect("previous");
        assert_eq!(world.borrow().active, Some(first));
        host.dispatch_workflow_command("previous")
            .expect("previous wrap");
        assert_eq!(world.borrow().active, Some(second));
        host.dispatch_workflow_command("next").expect("next");
        assert_eq!(world.borrow().active, Some(first));
        host.dispatch_workflow_command("next").expect("next wrap");
        assert_eq!(world.borrow().active, Some(second));
        host.dispatch_toolbar_toggle("reviewed").expect("reviewed");
        assert_eq!(
            world
                .borrow()
                .session
                .get(second)
                .and_then(|doc| doc.group.clone()),
            Some("reviewed".into())
        );
        let snapshot = host.toolbar_snapshot().expect("toolbar");
        assert_eq!(toggle_value(&snapshot.1[3]), Some(true));
        assert_eq!(message_text(&snapshot.1[5]), Some("1 of 2 files reviewed"));
        host.dispatch_workflow_command("next")
            .expect("skip reviewed");
        assert_eq!(world.borrow().active, Some(first));
        assert_eq!(
            toggle_value(&host.toolbar_snapshot().unwrap().1[3]),
            Some(false)
        );
        host.dispatch_workflow_command("drop").expect("drop");
        assert_eq!(
            world
                .borrow()
                .session
                .get(first)
                .and_then(|doc| doc.group.clone()),
            Some("drop".into())
        );
        assert_eq!(
            message_text(&host.toolbar_snapshot().unwrap().1[5]),
            Some("1 of 1 files reviewed")
        );
    }

    #[test]
    fn review_start_again_does_not_toggle_playback_off() {
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        host.invoke_menu_workflow("review").expect("review");
        assert!(world.borrow().looping);
        assert!(world.borrow().preview);
        assert!(world.borrow().explorer);
        host.invoke_menu_workflow("review").expect("again");
        assert!(world.borrow().looping);
        assert!(world.borrow().preview);
        assert!(world.borrow().explorer);
        host.finish_workflow().expect("finish");
        assert!(!world.borrow().looping);
        assert!(!world.borrow().preview);
        assert!(world.borrow().explorer);
    }

    #[test]
    fn view_pane_show_and_hide_are_idempotent() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:command("view.show-explorer")
            app:command("view.show-explorer")
            app:command("view.show-detail")
            app:command("view.hide-script")
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(world.borrow().explorer);
        assert!(world.borrow().detail);
        assert!(!world.borrow().script);
        let out = host.eval(
            r#"
            app:command("view.hide-explorer")
            app:command("view.hide-explorer")
            app:command("view.toggle-script")
            app:command("view.toggle-detail")
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(!world.borrow().explorer);
        assert!(!world.borrow().detail);
        assert!(world.borrow().script);
    }

    #[test]
    fn review_resume_shows_explorer() {
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("embedded init");
        let out = host.eval(r#"app.session.workflow = "review""#);
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(
            host.resume_workflow().expect("resume"),
            ResumeWorkflow::Resumed
        );
        assert!(world.borrow().explorer);
        assert!(world.borrow().looping);
        assert!(world.borrow().preview);
    }

    #[test]
    fn set_item_updates_toolbar_snapshot() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            local W = app:create_workflow({
              name = "stateful",
              display_name = "Stateful",
              scopes = { "drag-drop" },
            })
            function W:start(_payload)
              W:set_toolbar({
                { command = "go", label = "Go" },
                { kind = "message", id = "m", text = "a" },
              })
            end
            function W:suspend(_session) return true end
            W:on("command", function(command)
              if command == "go" then
                W:set_item("m", { text = "b" })
              end
            end)
            app:declare_workflow(W)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_workflow("stateful", &[]).expect("start");
        assert_eq!(
            message_text(&host.toolbar_snapshot().unwrap().1[1]),
            Some("a")
        );
        host.dispatch_workflow_command("go").expect("go");
        assert_eq!(
            message_text(&host.toolbar_snapshot().unwrap().1[1]),
            Some("b")
        );
    }

    #[test]
    fn oneshot_shorthand_does_not_bind_session_workflow() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            app:declare_workflow({
              name = "oneshot",
              scopes = { "drag-drop" },
            }, function(payload)
              app:info("oneshot", payload.scope)
            end)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_workflow("oneshot", &[PathBuf::from("take.wav")])
            .expect("invoke");
        assert!(world.borrow().session.workflow().is_none());
        assert!(host.active_workflow_name().is_none());
        assert!(host.toolbar_snapshot().is_none());
    }

    #[test]
    fn prototype_start_binds_and_finish_cancel_clear() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local W = app:create_workflow({
              name = "stateful",
              display_name = "Stateful",
              scopes = { "drag-drop" },
            })
            function W:start(_payload) end
            function W:suspend(_session) return true end
            function W:resume(_session) end
            W:set_toolbar({ { command = "go", label = "Go" } })
            app:declare_workflow(W)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_workflow("stateful", &[]).expect("start");
        assert_eq!(world.borrow().session.workflow(), Some("stateful"));
        assert_eq!(host.active_workflow_name().as_deref(), Some("stateful"));
        let snapshot = host.toolbar_snapshot().expect("toolbar");
        assert_eq!(snapshot.0, "Stateful");
        assert_eq!(snapshot.1[0].command(), Some("go"));
        host.finish_workflow().expect("finish");
        assert!(world.borrow().session.workflow().is_none());
        assert!(host.active_workflow_name().is_none());
        assert!(host.toolbar_snapshot().is_none());

        host.invoke_workflow("stateful", &[]).expect("start again");
        assert_eq!(world.borrow().session.workflow(), Some("stateful"));
        host.cancel_workflow().expect("cancel");
        assert!(world.borrow().session.workflow().is_none());
        assert!(host.active_workflow_name().is_none());
    }

    #[test]
    fn second_stateful_start_is_rejected_same_name_allowed() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local function make(name)
              local W = app:create_workflow({
                name = name,
                scopes = { "drag-drop" },
              })
              function W:start(_payload) end
              function W:suspend(_session) return true end
              app:declare_workflow(W)
            end
            make("alpha")
            make("beta")
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_workflow("alpha", &[]).expect("alpha");
        host.invoke_workflow("beta", &[]).expect("beta blocked");
        let alerts = host.take_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, "Cannot start workflow");
        assert_eq!(world.borrow().session.workflow(), Some("alpha"));
        host.invoke_workflow("alpha", &[]).expect("same name");
        assert_eq!(world.borrow().session.workflow(), Some("alpha"));
        assert!(host.take_alerts().is_empty());
    }

    #[test]
    fn suspend_false_is_reported_to_host() {
        let (mut host, _) = test_host();
        let out = host.eval(
            r#"
            local W = app:create_workflow({
              name = "block",
              scopes = { "drag-drop" },
            })
            function W:start(_payload) end
            function W:suspend(_session) return false end
            app:declare_workflow(W)
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        host.invoke_workflow("block", &[]).expect("start");
        let allowed = host.suspend_workflow().expect("suspend");
        assert!(!allowed);
    }

    #[test]
    fn resume_restores_stateful_workflow_from_session() {
        let (mut host, world) = test_host();
        let out = host.eval(
            r#"
            local W = app:create_workflow({
              name = "stateful",
              display_name = "Stateful",
              scopes = { "drag-drop" },
            })
            function W:start(_payload) end
            function W:suspend(_session) return true end
            function W:resume(session)
              app:info("stateful", session.properties.note or "")
              W:set_toolbar({ { command = "go", label = "Go" } })
            end
            app:declare_workflow(W)
            app.session.workflow = "stateful"
            app.session.properties = { note = "hello" }
            "#,
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(world.borrow().session.workflow(), Some("stateful"));
        let result = host.resume_workflow().expect("resume");
        assert_eq!(result, ResumeWorkflow::Resumed);
        let logs = host.take_logs();
        assert!(
            logs.iter().any(|entry| entry.message == "hello"),
            "{logs:?}"
        );
        let snapshot = host.toolbar_snapshot().expect("toolbar");
        assert_eq!(snapshot.0, "Stateful");
        assert_eq!(snapshot.1[0].label(), Some("Go"));
    }

    #[test]
    fn unknown_resume_logs_error_and_does_not_clear() {
        let (mut host, world) = test_host();
        let out = host.eval(r#"app.session.workflow = "ghost""#);
        assert!(out.error.is_none(), "{:?}", out.error);
        let result = host.resume_workflow().expect("resume");
        assert_eq!(
            result,
            ResumeWorkflow::Unknown {
                name: "ghost".into()
            }
        );
        assert_eq!(world.borrow().session.workflow(), Some("ghost"));
        assert!(host.active_workflow_name().is_none());
        assert!(host.toolbar_snapshot().is_none());
        let logs = host.take_logs();
        assert!(
            logs.iter()
                .any(|entry| { entry.level == LogLevel::Error && entry.message.contains("ghost") }),
            "{logs:?}"
        );
    }

    #[test]
    fn user_workflow_file_overrides_builtin() {
        let dir = std::env::temp_dir().join("fieldassist-workflow-override");
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join("init.lua"), "app:info('init', 'user')\n").expect("init");
        std::fs::write(
            dir.join("workflow_add.lua"),
            r#"
            app:declare_workflow({
              name = "add",
              display_name = "Custom Add",
              scopes = { "drag-drop" },
              drop = { row = 1, priority = 1 },
            }, function() end)
            "#,
        )
        .expect("workflow");
        let (mut host, _) = test_host();
        host.load_init_from(Some(&dir)).expect("user workflows");
        let add = host
            .workflow_metas()
            .into_iter()
            .find(|meta| meta.name == "add")
            .expect("add");
        assert_eq!(add.display_name, "Custom Add");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_session_does_not_replace_active() {
        let dir = std::env::temp_dir().join("fieldassist-load-session");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let wav = dir.join("take.wav");
        std::fs::write(&wav, b"wav").expect("wav");
        let session_path = dir.join("batch.fasession");
        let mut incoming = crate::model::Session::new();
        incoming.insert(crate::model::SessionDocument::new(
            crate::model::DocumentId::from_u128(9),
            Some(wav.clone()),
        ));
        incoming
            .save_to_path(&session_path, |_| "take.wav".into(), None)
            .expect("save session");
        let (mut host, world) = test_host();
        let active_id = world.borrow().session.id().to_string();
        let path_lua = session_path.to_string_lossy().replace('\\', "/");
        let out = host.eval(&format!(
            r#"
            local incoming = app:load_session("{path_lua}")
            local active = app.session.id
            incoming:close()
            return incoming.id ~= active, active, #app.sessions
            "#
        ));
        assert!(out.error.is_none(), "{:?}", out.error);
        assert!(
            out.result
                .as_deref()
                .is_some_and(|value| value.starts_with("true")),
            "{:?}",
            out.result
        );
        assert_eq!(world.borrow().session.id().to_string(), active_id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_workflow_merges_session_and_skips_matching_path() {
        let dir = std::env::temp_dir().join("fieldassist-add-merge");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let existing = dir.join("keep.wav");
        let extra = dir.join("extra.wav");
        std::fs::write(&existing, b"a").expect("existing");
        std::fs::write(&extra, b"b").expect("extra");
        let session_path = dir.join("incoming.fasession");
        let mut incoming = crate::model::Session::new();
        incoming.insert(crate::model::SessionDocument::new(
            crate::model::DocumentId::from_u128(11),
            Some(existing.clone()),
        ));
        incoming.insert(crate::model::SessionDocument::new(
            crate::model::DocumentId::from_u128(12),
            Some(extra.clone()),
        ));
        incoming
            .save_to_path(
                &session_path,
                |id| {
                    if id == crate::model::DocumentId::from_u128(11) {
                        "keep.wav".into()
                    } else {
                        "extra.wav".into()
                    }
                },
                None,
            )
            .expect("save session");
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("workflows");
        let keep_id = world.borrow_mut().push(
            crate::model::composition::Composition::new(44100, 2),
            crate::model::Buffer::empty(),
            "keep.wav",
            Some(existing.clone()),
        );
        let before = world.borrow().session.documents().len();
        let err = host.invoke_workflow("add", &[session_path.clone()]);
        assert!(err.is_ok(), "{err:?}");
        let world = world.borrow();
        assert_eq!(world.session.documents().len(), before + 1);
        assert!(world.session.get(keep_id).is_some());
        assert!(world.session.find_by_path(&extra).is_some());
        let _ = keep_id;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_workflow_alerts_on_multiple_files() {
        let (mut host, _) = test_host();
        host.load_init_from(None).expect("workflows");
        host.invoke_workflow("replace", &[PathBuf::from("a.wav"), PathBuf::from("b.wav")])
            .expect("invoke");
        let alerts = host.take_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].0, "Cannot replace");
    }

    #[test]
    fn replace_workflow_replaces_active_document() {
        let dir = std::env::temp_dir().join("fieldassist-replace-doc");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let next = dir.join("next.wav");
        std::fs::write(&next, b"wav").expect("wav");
        let (mut host, world) = test_host();
        host.load_init_from(None).expect("workflows");
        let id = world.borrow().active.expect("active");
        host.invoke_workflow("replace", &[next.clone()])
            .expect("invoke");
        let world = world.borrow();
        assert_eq!(world.paths.get(&id).cloned().flatten(), Some(next.clone()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
