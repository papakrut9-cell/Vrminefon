//! VR's single-pass stereo: the world pass's first part (sky, solid terrain, block
//! entities and entities, the part VR's MSAA draws multisampled) drawn once for both
//! eyes through Vulkan multiview, into two-layer targets, instead of once per eye.
//! The Quest 3's GPU spends most of a frame on that part's draws (measured 2026-11-18:
//! ~600 draws and 11.5 ms per eye), and multiview records and bins them once.
//!
//! The left eye's frame draws both layers; each eye's frame then takes its own layer
//! (colour copied into the world target, depth through MSAA's depth copy) and draws the
//! rest of its world (translucent terrain, particles, clouds...) as before. The right
//! eye's frame skips the first part.
//!
//! The shaders are the usual ones, rewritten (`multiview_wgsl`): every entry point
//! takes the view index, and the per-eye uniforms are read from the frame's two eyes
//! (`Globals::stereo_*`) instead of the one view.

/// Off by default since flat terrain draws (measured 2026-11-19 on the Quest 3: with them
/// the multiview pass cost ~14 ms for both eyes, two plain passes ~9 ms, and translucent
/// terrain only draws multisampled one eye at a time). `HORIZON_XR_STEREO=1` turns it on.
pub(super) fn wanted() -> bool {
    std::env::var("HORIZON_XR_STEREO").as_deref() == Ok("1")
}

/// The multiview copy of a WGSL `source`: each `(from, to)` replaced (`to` may read
/// `mv_view`, the view being drawn), and every vertex and fragment entry point given the
/// view index, stored in the private `mv_view` for the functions it calls.
pub(super) fn multiview_wgsl(source: &str, replace: &[(&str, &str)]) -> String {
    let mut text = source.to_owned();
    for (from, to) in replace {
        text = text.replace(from, to);
    }
    let mut out = String::with_capacity(text.len() + 4096);
    out.push_str("var<private> mv_view: u32;\n");
    let mut rest = text.as_str();
    loop {
        let next = ["@vertex", "@fragment"]
            .iter()
            .filter_map(|tag| rest.find(tag).map(|at| (at, tag.len())))
            .min();
        let Some((at, tag_len)) = next else {
            out.push_str(rest);
            break;
        };
        // `@vertex fn name(`: the parameters open at the first `(` after the tag.
        let after_tag = at + tag_len;
        let Some(open) = rest[after_tag..].find('(').map(|o| after_tag + o) else {
            out.push_str(rest);
            break;
        };
        let header = &rest[after_tag..open];
        if !header.trim_start().starts_with("fn ") {
            out.push_str(&rest[..after_tag]);
            rest = &rest[after_tag..];
            continue;
        }
        // The matching `)`, then the body's `{`.
        let mut depth = 0usize;
        let mut close = None;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else {
            out.push_str(rest);
            break;
        };
        let Some(body) = rest[close..].find('{').map(|b| close + b) else {
            out.push_str(rest);
            break;
        };
        let params = rest[open + 1..close].trim();
        out.push_str(&rest[..=open]);
        out.push_str("@builtin(view_index) mv_view_in: u32");
        if !params.is_empty() {
            out.push_str(", ");
        }
        out.push_str(&rest[open + 1..=body]);
        out.push_str("\n    mv_view = mv_view_in;");
        rest = &rest[body + 1..];
    }
    out
}

/// The terrain and entity shaders' per-eye uniforms, read from the drawn eye's.
pub(super) const GLOBALS: &[(&str, &str)] = &[
    ("globals.view_proj", "globals.stereo_view_proj[mv_view]"),
    ("globals.inverse_view", "globals.stereo_inverse_view[mv_view]"),
    ("globals.misc.z", "globals.stereo_lens[mv_view].x"),
    ("globals.misc.w", "globals.stereo_lens[mv_view].y"),
];

/// The stars' clip matrix, the drawn eye's.
pub(super) const STARS: &[(&str, &str)] = &[("stars.clip_from_sky", "stars.clip_from_sky_eyes[mv_view]")];

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str) {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
        for entry in &module.entry_points {
            if matches!(entry.stage, naga::ShaderStage::Vertex | naga::ShaderStage::Fragment) {
                assert!(
                    entry.function.arguments.iter().any(|a| matches!(
                        a.binding,
                        Some(naga::Binding::BuiltIn(naga::BuiltIn::ViewIndex))
                    )),
                    "{} takes no view index",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn the_multiview_shaders_validate_with_the_view_index_everywhere() {
        let terrain = multiview_wgsl(include_str!("../terrain.wgsl"), GLOBALS);
        assert!(!terrain.contains("globals.view_proj"));
        assert!(!terrain.contains("globals.inverse_view"));
        validate(&terrain);
        validate(&multiview_wgsl(&super::super::entity_shader(), GLOBALS));
        validate(&multiview_wgsl(&super::super::static_entity_shader(), GLOBALS));
        let stars = multiview_wgsl(include_str!("stars.wgsl"), STARS);
        validate(&stars);
    }

    #[test]
    fn entry_points_without_parameters_take_the_view_index_alone() {
        let out = multiview_wgsl("@fragment fn fs() -> @location(0) vec4<f32> { return vec4(1.0); }", &[]);
        assert!(out.contains("fn fs(@builtin(view_index) mv_view_in: u32) -> @location(0) vec4<f32> {\n    mv_view = mv_view_in;"));
    }
}
