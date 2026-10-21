use crate::{layout::KeyGeometry, scene::Camera};
use eframe::egui::{self, Color32, FontId, Pos2, Rect, Shape, Stroke, Vec2};
use std::sync::Arc;

const CENTER: Vec2 = Vec2::new(345.0, 155.0);
pub const FACE_HEIGHT: f32 = 12.0;

#[derive(Clone, Copy)]
pub struct BoardView {
    pub area: Rect,
    pub camera: Camera,
    pub scale: f32,
    origin: Pos2,
}
impl BoardView {
    pub fn new(area: Rect, camera: Camera) -> Self {
        let mut bounds = Rect::NOTHING;
        for x in [-345.0, 345.0] {
            for y in [-155.0, 155.0] {
                for z in [-18.0, 20.0] {
                    bounds.extend_with(camera.project_height(Vec2::new(x, y), z).to_pos2());
                }
            }
        }
        let scale = ((area.width() - 38.0) / bounds.width())
            .min((area.height() - 40.0) / bounds.height())
            .max(0.1);
        Self {
            area,
            camera,
            scale,
            origin: area.center() - bounds.center().to_vec2() * scale,
        }
    }
    pub fn point(self, point: Pos2, height: f32) -> Pos2 {
        let local = point.to_vec2() - CENTER;
        let projected = if height == 0.0 {
            self.camera.project(local)
        } else {
            self.camera.project_height(local, height)
        };
        self.origin + projected * self.scale
    }
    pub fn local(self, point: Pos2, height: f32) -> Pos2 {
        let projected = (point - self.origin) / self.scale;
        let local = if height == 0.0 {
            self.camera.unproject(projected)
        } else {
            self.camera.unproject_height(projected, height)
        };
        (local + CENTER).to_pos2()
    }
    fn depth(self, point: Pos2, height: f32) -> f32 {
        self.camera.depth(point.to_vec2() - CENTER, height)
    }
}

pub struct SceneKey<'a> {
    pub geometry: KeyGeometry,
    pub matrix: usize,
    pub label: &'a str,
    pub rgb: [u8; 3],
}

pub fn hit_key(view: BoardView, keys: &[SceneKey<'_>], point: Pos2) -> Option<usize> {
    let local = view.local(point, FACE_HEIGHT);
    // Resolve the same surfaces and back-to-front order that paint uses, including tall sidewalls.
    keys.iter()
        .filter(|key| {
            let foot = key_path(key.geometry, 0.0, |p| view.point(p, 1.8));
            let shoulder = key_path(key.geometry, 1.0, |p| view.point(p, FACE_HEIGHT - 3.0));
            let top = key_path(key.geometry, 3.2, |p| view.point(p, FACE_HEIGHT));
            (key.geometry.contains(local) && contains_polygon(&top, point))
                || [(&foot, &shoulder), (&shoulder, &top)]
                    .into_iter()
                    .any(|(lower, upper)| {
                        (0..lower.len()).any(|i| {
                            let next = (i + 1) % lower.len();
                            contains_polygon(&[lower[i], lower[next], upper[next], upper[i]], point)
                        })
                    })
        })
        .max_by(|a, b| {
            view.depth(a.geometry.rect.center(), 0.0)
                .total_cmp(&view.depth(b.geometry.rect.center(), 0.0))
        })
        .map(|key| key.matrix)
}

fn contains_polygon(points: &[Pos2], point: Pos2) -> bool {
    let mut inside = false;
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

pub fn paint(
    painter: &egui::Painter,
    view: BoardView,
    keys: &[SceneKey<'_>],
    selected: usize,
    hovered: Option<usize>,
    pressed: bool,
    lit: bool,
) {
    let painter = &painter.with_clip_rect(view.area);
    paint_stage_light(painter, view.area);
    let shell = Rect::from_min_size(Pos2::ZERO, Vec2::new(690.0, 310.0));
    // Broad, low-opacity layers give a contact shadow without a continuous effect loop.
    for layer in (0..8).rev() {
        let points = rounded_board_path(shell.expand(layer as f32 * 1.8), 26.0, |p| {
            view.point(p, -18.0) + Vec2::new(0.0, 8.0 + layer as f32)
        });
        painter.add(polygon_face(&points, Color32::from_black_alpha(10)));
    }
    let lower = rounded_board_path(shell, 25.0, |p| view.point(p, -14.0));
    let upper = rounded_board_path(shell, 25.0, |p| view.point(p, 0.0));
    walls(
        painter,
        &lower,
        &upper,
        Color32::from_rgb(10, 13, 18),
        Color32::from_rgb(25, 30, 37),
    );
    painter.add(shaded_face(
        &upper,
        Color32::from_rgb(39, 45, 54),
        Color32::from_rgb(20, 25, 31),
    ));
    painter.add(Shape::closed_line(
        upper,
        Stroke::new(0.8, Color32::from_rgb(66, 75, 84)),
    ));
    let lip = rounded_board_path(shell.shrink(10.0), 18.0, |p| view.point(p, 0.5));
    painter.add(polygon_face(&lip, Color32::from_rgb(6, 9, 14)));
    let plate = rounded_board_path(shell.shrink(14.0), 14.0, |p| view.point(p, 0.9));
    painter.add(shaded_face(
        &plate,
        Color32::from_rgb(14, 18, 25),
        Color32::from_rgb(8, 12, 18),
    ));

    if lit {
        for key in keys {
            for (spread, alpha) in [(9.0, 13), (5.0, 28), (1.5, 90)] {
                let path = key_path(key.geometry, -spread, |p| view.point(p, 1.0));
                painter.add(polygon_face(
                    &path,
                    Color32::from_rgba_unmultiplied(key.rgb[0], key.rgb[1], key.rgb[2], alpha),
                ));
            }
        }
    }
    let mut sorted = keys.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        view.depth(a.geometry.rect.center(), 0.0)
            .total_cmp(&view.depth(b.geometry.rect.center(), 0.0))
    });
    for key in sorted {
        let hover = painter.ctx().animate_bool_with_time(
            egui::Id::new(("key-lift", key.matrix)),
            hovered == Some(key.matrix),
            0.12,
        );
        let push = painter.ctx().animate_bool_with_time(
            egui::Id::new(("key-push", key.matrix)),
            pressed && hovered == Some(key.matrix),
            0.08,
        );
        let height = FACE_HEIGHT + hover * 0.8 - push * 1.8;
        let gold = matches!(
            key.label,
            "Esc" | "Enter" | "Space" | "Up" | "Down" | "Left" | "Right"
        );
        let modifier = matches!(
            key.label,
            "Tab"
                | "CapsLock"
                | "LShift"
                | "RShift"
                | "LCtrl"
                | "LWin"
                | "LAlt"
                | "RAlt"
                | "FN"
                | "Backspace"
        );
        let fill = if gold {
            Color32::from_rgb(243, 184, 54)
        } else if modifier {
            Color32::from_rgb(35, 43, 55)
        } else {
            Color32::from_rgb(61, 70, 92)
        };
        let fill = mix_color(fill, Color32::from_rgb(112, 141, 155), hover * 0.12);
        let foot = key_path(key.geometry, 0.0, |p| view.point(p, 1.8));
        let shoulder = key_path(key.geometry, 1.0, |p| view.point(p, height - 3.0));
        let top = key_path(key.geometry, 3.2, |p| view.point(p, height));
        let dark = mix_color(fill, Color32::BLACK, 0.55);
        walls(
            painter,
            &foot,
            &shoulder,
            dark,
            mix_color(fill, Color32::BLACK, 0.25),
        );
        walls(
            painter,
            &shoulder,
            &top,
            mix_color(fill, Color32::BLACK, 0.22),
            mix_color(fill, Color32::WHITE, 0.12),
        );
        painter.add(shaded_face(
            &top,
            mix_color(fill, Color32::WHITE, 0.09),
            mix_color(fill, Color32::BLACK, 0.12),
        ));
        painter.add(Shape::closed_line(
            top,
            Stroke::new(0.65, mix_color(fill, Color32::WHITE, 0.19)),
        ));
        if selected == key.matrix {
            let outline = key_path(key.geometry, -1.0, |p| view.point(p, 2.0));
            painter.add(Shape::closed_line(
                outline,
                Stroke::new(1.3, Color32::from_rgb(57, 231, 139)),
            ));
        }
        let color = if gold {
            Color32::from_rgb(49, 37, 15)
        } else {
            Color32::from_rgb(224, 222, 208)
        };
        let center = key.geometry.legend_center();
        let direction = match key.label {
            "Up" => Some(Vec2::new(0.0, -1.0)),
            "Down" => Some(Vec2::Y),
            "Left" => Some(Vec2::new(-1.0, 0.0)),
            "Right" => Some(Vec2::X),
            _ => None,
        };
        if let Some(direction) = direction {
            let tip = center + direction * 4.6;
            let normal = Vec2::new(-direction.y, direction.x);
            let stroke = Stroke::new((1.05 * view.scale).clamp(0.8, 1.8), color);
            painter.line_segment(
                [
                    view.point(center - direction * 4.6, height + 0.2),
                    view.point(tip, height + 0.2),
                ],
                stroke,
            );
            painter.add(Shape::line(
                vec![
                    view.point(tip - direction * 3.0 + normal * 3.0, height + 0.2),
                    view.point(tip, height + 0.2),
                    view.point(tip - direction * 3.0 - normal * 3.0, height + 0.2),
                ],
                stroke,
            ));
        } else {
            let text = match key.label {
                "CapsLock" => "Caps",
                "LShift" | "RShift" => "Shift",
                "LCtrl" => "Ctrl",
                "LWin" => "Win",
                "LAlt" | "RAlt" => "Alt",
                label => label,
            };
            paint_legend(painter, view, text, center, height + 0.2, color);
        }
    }
    knob(painter, view);
}

fn walls(painter: &egui::Painter, lower: &[Pos2], upper: &[Pos2], bottom: Color32, top: Color32) {
    let mut mesh = egui::Mesh::default();
    for edge in 0..lower.len() {
        let next = (edge + 1) % lower.len();
        let offset = mesh.vertices.len() as u32;
        // Vertex colours light the taper. Explicit triangles avoid thin-polygon AA spikes.
        mesh.colored_vertex(lower[edge], bottom);
        mesh.colored_vertex(lower[next], bottom);
        mesh.colored_vertex(upper[next], top);
        mesh.colored_vertex(upper[edge], top);
        mesh.indices.extend_from_slice(&[
            offset,
            offset + 1,
            offset + 2,
            offset,
            offset + 2,
            offset + 3,
        ]);
    }
    painter.add(Shape::mesh(mesh));
}

fn knob(painter: &egui::Painter, view: BoardView) {
    let center = Pos2::new(640.0, 45.0);
    let ring = |radius: f32, height: f32| {
        (0..64)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / 64.0;
                view.point(center + Vec2::angled(a) * radius, height)
            })
            .collect::<Vec<_>>()
    };
    painter.add(polygon_face(&ring(26.0, 0.5), Color32::from_rgb(2, 5, 8)));
    painter.add(Shape::closed_line(
        ring(24.5, 1.5),
        Stroke::new(1.5, Color32::from_rgb(55, 218, 132)),
    ));
    walls(
        painter,
        &ring(22.0, 2.0),
        &ring(22.0, 13.0),
        Color32::from_rgb(46, 54, 65),
        Color32::from_rgb(113, 125, 137),
    );
    walls(
        painter,
        &ring(22.0, 13.0),
        &ring(19.0, 16.0),
        Color32::from_rgb(89, 101, 111),
        Color32::from_rgb(190, 199, 204),
    );
    let outer = ring(19.0, 16.0);
    let inner = ring(14.2, 16.0);
    walls(
        painter,
        &outer,
        &inner,
        Color32::from_rgb(187, 194, 199),
        Color32::from_rgb(99, 113, 123),
    );
    painter.add(shaded_face(
        &inner,
        Color32::from_rgb(30, 38, 46),
        Color32::from_rgb(16, 22, 29),
    ));
    for i in 0..40 {
        let a = i as f32 * std::f32::consts::TAU / 40.0;
        let p = center + Vec2::angled(a) * 22.0;
        if view.depth(p, 10.0) > view.depth(center, 10.0) {
            painter.line_segment(
                [view.point(p, 4.0), view.point(p, 11.0)],
                Stroke::new(0.65, Color32::from_rgb(34, 42, 52)),
            );
        }
    }
    painter.line_segment(
        [
            view.point(center + Vec2::new(3.0, 4.0), 16.2),
            view.point(center + Vec2::new(8.0, 10.0), 16.2),
        ],
        Stroke::new(1.7, Color32::from_rgb(86, 216, 155)),
    );
}

fn paint_legend(
    painter: &egui::Painter,
    view: BoardView,
    text: &str,
    center: Pos2,
    height: f32,
    color: Color32,
) {
    let font_size = (11.5 * view.scale).clamp(7.0, 18.0);
    let source = painter.layout_no_wrap(text.into(), FontId::proportional(font_size), color);
    let mut galley = (*source).clone();
    let anchor = view.point(center, height);
    let local_origin = center - source.size() / view.scale * 0.5;
    let mut bounds = Rect::NOTHING;
    for placed in &mut galley.rows {
        let row_offset = placed.pos.to_vec2();
        let row = Arc::make_mut(&mut placed.row);
        let mut row_bounds = Rect::NOTHING;
        for vertex in &mut row.visuals.mesh.vertices {
            let local = local_origin + (vertex.pos.to_vec2() + row_offset) / view.scale;
            vertex.pos = (view.point(local, height) - anchor).to_pos2();
            row_bounds.extend_with(vertex.pos);
        }
        placed.pos = Pos2::ZERO;
        row.visuals.mesh_bounds = row_bounds;
        bounds = bounds.union(row_bounds);
    }
    galley.mesh_bounds = bounds;
    galley.rect = bounds;
    // Retain egui's font atlas, colour handling and text metadata while projecting glyph vertices.
    painter.add(egui::epaint::TextShape::new(
        anchor,
        Arc::new(galley),
        color,
    ));
}

pub fn mix_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let amount = amount.clamp(0.0, 1.0);
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    Color32::from_rgb(
        blend(from.r(), to.r()),
        blend(from.g(), to.g()),
        blend(from.b(), to.b()),
    )
}
pub fn key_path(
    geometry: KeyGeometry,
    inset: f32,
    transform: impl Fn(egui::Pos2) -> egui::Pos2,
) -> Vec<egui::Pos2> {
    if geometry.iso_enter {
        let points = geometry.outline(inset);
        let mut rounded = Vec::new();
        for index in 0..points.len() {
            let corner = points[index];
            let entry = corner
                + (points[(index + points.len() - 1) % points.len()] - corner).normalized() * 2.5;
            let exit = corner + (points[(index + 1) % points.len()] - corner).normalized() * 2.5;
            for step in 0..=4 {
                let t = step as f32 / 4.0;
                rounded.push(transform(
                    (entry.to_vec2() * (1.0 - t).powi(2)
                        + corner.to_vec2() * 2.0 * (1.0 - t) * t
                        + exit.to_vec2() * t * t)
                        .to_pos2(),
                ));
            }
        }
        rounded
    } else {
        rounded_board_path(geometry.rect.shrink(inset), 4.0, transform)
    }
}
// Enter is concave. Ear clipping preserves its notch instead of filling the adjacent key.
pub fn polygon_triangles(points: &[egui::Pos2]) -> Vec<u32> {
    let cross = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
        let u = b - a;
        let v = c - a;
        u.x * v.y - u.y * v.x
    };
    let mut remaining = (0..points.len()).collect::<Vec<_>>();
    let mut triangles = Vec::new();
    while remaining.len() > 2 {
        let ear = (0..remaining.len()).find(|&i| {
            let a = remaining[(i + remaining.len() - 1) % remaining.len()];
            let b = remaining[i];
            let c = remaining[(i + 1) % remaining.len()];
            cross(points[a], points[b], points[c]) > 0.0001
                && !remaining.iter().any(|&p| {
                    p != a
                        && p != b
                        && p != c
                        && cross(points[a], points[b], points[p]) >= -0.0001
                        && cross(points[b], points[c], points[p]) >= -0.0001
                        && cross(points[c], points[a], points[p]) >= -0.0001
                })
        });
        let Some(i) = ear else { break };
        triangles.extend_from_slice(&[
            remaining[(i + remaining.len() - 1) % remaining.len()] as u32,
            remaining[i] as u32,
            remaining[(i + 1) % remaining.len()] as u32,
        ]);
        remaining.remove(i);
    }
    triangles
}
fn polygon_face(points: &[egui::Pos2], color: Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    for point in points {
        mesh.colored_vertex(*point, color);
    }
    mesh.indices = polygon_triangles(points);
    egui::Shape::mesh(mesh)
}
fn shaded_face(points: &[egui::Pos2], top: Color32, bottom: Color32) -> egui::Shape {
    let bounds = egui::Rect::from_points(points);
    let mut mesh = egui::Mesh::default();
    for point in points {
        mesh.colored_vertex(
            *point,
            mix_color(
                top,
                bottom,
                (point.y - bounds.top()) / bounds.height().max(1.0),
            ),
        );
    }
    mesh.indices = polygon_triangles(points);
    egui::Shape::mesh(mesh)
}
fn paint_stage_light(painter: &egui::Painter, area: egui::Rect) {
    let mut mesh = egui::Mesh::default();
    let center = area.center();
    mesh.colored_vertex(center, Color32::from_rgb(19, 35, 28));
    for step in 0..=48 {
        let angle = step as f32 * std::f32::consts::TAU / 48.0;
        mesh.colored_vertex(
            center
                + Vec2::new(
                    angle.cos() * area.width() * 0.49,
                    angle.sin() * area.height() * 0.48,
                ),
            Color32::from_rgb(7, 11, 9),
        );
        if step > 0 {
            mesh.indices.extend_from_slice(&[0, step, step + 1]);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}
fn rounded_board_path(
    rect: egui::Rect,
    radius: f32,
    transform: impl Fn(egui::Pos2) -> egui::Pos2,
) -> Vec<egui::Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let mut points = Vec::with_capacity(20);
    for (center, start) in [
        (rect.left_top() + Vec2::splat(radius), std::f32::consts::PI),
        (
            rect.right_top() + Vec2::new(-radius, radius),
            1.5 * std::f32::consts::PI,
        ),
        (rect.right_bottom() - Vec2::splat(radius), 0.0),
        (
            rect.left_bottom() + Vec2::new(radius, -radius),
            0.5 * std::f32::consts::PI,
        ),
    ] {
        for step in 0..=4 {
            let angle = start + step as f32 * std::f32::consts::FRAC_PI_2 / 4.0;
            points.push(transform(
                center + Vec2::new(angle.cos(), angle.sin()) * radius,
            ));
        }
    }
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_fit_contains_case_and_raised_caps_in_all_supported_views() {
        let keys: serde_json::Value =
            serde_json::from_str(include_str!("../assets/keyboard-layout.json")).unwrap();
        for size in [Vec2::new(1000.0, 555.0), Vec2::new(474.0, 285.0)] {
            let area = Rect::from_min_size(Pos2::new(116.0, 140.0), size);
            for yaw in [-0.42, -0.20, 0.42] {
                for pitch in [0.0, 0.78, 0.95] {
                    let view = BoardView::new(area, Camera { yaw, pitch });
                    for key in keys.as_array().unwrap() {
                        let number = |field| key[field].as_f64().unwrap() as f32;
                        let Some(geometry) = crate::layout::from_oem(
                            number("x"),
                            number("y"),
                            number("width"),
                            number("height"),
                            key["matrix"].as_u64().unwrap() as usize,
                        ) else {
                            continue;
                        };
                        for point in geometry.outline(0.0) {
                            for height in [1.8, 12.8] {
                                let projected = view.point(point, height);
                                assert!(
                                    area.shrink(6.0).contains(projected),
                                    "Raised cap clipped at {projected:?}"
                                );
                                assert!((view.local(projected, height) - point).length() < 0.001);
                            }
                        }
                    }
                    for point in [Pos2::ZERO, Pos2::new(690.0, 310.0)] {
                        assert!(area.shrink(6.0).contains(view.point(point, -14.0)));
                    }
                }
            }
        }
    }

    #[test]
    fn projected_glyph_vertices_stay_on_the_key_surface() {
        let context = egui::Context::default();
        let view = BoardView::new(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 555.0)),
            Camera::default(),
        );
        let key = crate::layout::from_oem(547.0, 88.0, 66.0, 28.0, 79).unwrap();
        let mut output = context.run_ui(egui::RawInput::default(), |ui| {
            paint_legend(
                ui.painter(),
                view,
                "Backspace",
                key.legend_center(),
                12.2,
                Color32::WHITE,
            );
        });
        output.textures_delta.clear();
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Text(text) => Some(text),
                _ => None,
            })
            .unwrap();
        for row in &text.galley.rows {
            for vertex in &row.visuals.mesh.vertices {
                let board = view.local(text.pos + vertex.pos.to_vec2(), 12.2);
                assert!(
                    key.rect.shrink(2.0).contains(board),
                    "Legend left key face: {board:?}"
                );
            }
        }
    }

    #[test]
    fn visible_key_sidewall_is_selectable_with_perspective() {
        let geometry = crate::layout::from_oem(67.0, 88.0, 26.0, 28.0, 7).unwrap();
        let key = SceneKey {
            geometry,
            matrix: 7,
            label: "1",
            rgb: [0, 255, 0],
        };
        let view = BoardView::new(
            Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 555.0)),
            Camera::default(),
        );
        let wall_point = view.point(Pos2::new(80.0, 116.97), 2.0);
        assert!(!geometry.contains(view.local(wall_point, FACE_HEIGHT)));
        assert_eq!(hit_key(view, &[key], wall_point), Some(7));
    }

    #[test]
    fn tessellated_scene_has_no_long_spikes_at_steep_camera_angles() {
        let imported: serde_json::Value =
            serde_json::from_str(include_str!("../assets/keyboard-layout.json")).unwrap();
        let keys = imported
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|key| {
                let number = |field| key[field].as_f64().unwrap() as f32;
                let matrix = key["matrix"].as_u64().unwrap() as usize;
                crate::layout::from_oem(
                    number("x"),
                    number("y"),
                    number("width"),
                    number("height"),
                    matrix,
                )
                .map(|geometry| SceneKey {
                    geometry,
                    matrix,
                    label: key["label"].as_str().unwrap(),
                    rgb: [36, 180, 240],
                })
            })
            .collect::<Vec<_>>();
        for yaw in [-0.42, 0.42] {
            let context = egui::Context::default();
            let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 555.0));
            let view = BoardView::new(area, Camera { yaw, pitch: 0.95 });
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(area),
                    ..Default::default()
                },
                |ui| paint(ui.painter(), view, &keys, 0, None, false, true),
            );
            output.textures_delta.clear();
            for primitive in context.tessellate(output.shapes, output.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    for vertex in mesh.vertices {
                        assert!(
                            area.expand(12.0).contains(vertex.pos),
                            "Rendering spike at {:?}, yaw {yaw}",
                            vertex.pos
                        );
                    }
                }
            }
        }
    }
}
