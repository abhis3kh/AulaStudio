use eframe::egui::{Pos2, Rect, Vec2};

/// A key's visible face in the shared, normalized keyboard coordinate system.
/// Hardware matrix identities remain those of the imported OEM layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyGeometry {
    pub rect: Rect,
    pub iso_enter: bool,
}

/// Convert the OEM hit rectangles into aligned keycap faces. The OEM image uses
/// a 40 px key pitch, but its hit rectangles have small, irregular pixel offsets.
pub fn from_oem(x: f32, y: f32, width: f32, height: f32, matrix: usize) -> Option<KeyGeometry> {
    if ![x, y, width, height].iter().all(|value| value.is_finite()) || width <= 0.0 || height <= 0.0
    {
        return None;
    }

    if matrix == 81 {
        // The OEM's UnrectKey entry describes this ISO Enter's missing notch.
        return Some(KeyGeometry {
            rect: Rect::from_min_max(Pos2::new(563.0, 123.0), Pos2::new(617.0, 197.0)),
            iso_enter: true,
        });
    }

    let source_rows = [34.0_f32, 88.0, 128.0, 168.0, 208.0, 248.0];
    let aligned_rows = [24.0, 80.0, 120.0, 160.0, 200.0, 240.0];
    let row = source_rows
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (y - **a).abs().total_cmp(&(y - **b).abs()))
        .map(|(index, _)| index)?;
    let cell_x = 20.0 + ((x - 27.0) / 10.0).round() * 10.0;
    let cell_width = ((width + 14.0) / 10.0).round() * 10.0;
    if cell_width <= 6.0 {
        return None;
    }
    Some(KeyGeometry {
        rect: Rect::from_min_size(
            Pos2::new(cell_x + 3.0, aligned_rows[row] + 3.0),
            Vec2::new(cell_width - 6.0, 34.0),
        ),
        iso_enter: false,
    })
}

impl KeyGeometry {
    pub fn contains(self, point: Pos2) -> bool {
        self.rect.contains(point) && (!self.iso_enter || point.x >= 573.0 || point.y <= 157.0)
    }

    /// Face outline, with positive values insetting and negative values expanding.
    /// The Enter outline is concave, so its triangles must not use a convex fan.
    pub fn outline(self, inset: f32) -> Vec<Pos2> {
        let inset = inset.min(self.rect.width().min(34.0) * 0.5 - 0.001);
        let rect = self.rect.shrink(inset);
        if self.iso_enter {
            vec![
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                Pos2::new(573.0 + inset, rect.bottom()),
                Pos2::new(573.0 + inset, 157.0 - inset),
                Pos2::new(rect.left(), 157.0 - inset),
            ]
        } else {
            vec![
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
            ]
        }
    }

    pub fn legend_center(self) -> Pos2 {
        if self.iso_enter {
            Pos2::new(595.0, 176.0)
        } else {
            self.rect.center()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imported_geometry() -> Vec<(usize, KeyGeometry)> {
        let keys: serde_json::Value =
            serde_json::from_str(include_str!("../assets/keyboard-layout.json")).unwrap();
        keys.as_array()
            .unwrap()
            .iter()
            .filter_map(|key| {
                let matrix = key["matrix"].as_u64().unwrap() as usize;
                let number = |name| key[name].as_f64().unwrap() as f32;
                from_oem(
                    number("x"),
                    number("y"),
                    number("width"),
                    number("height"),
                    matrix,
                )
                .map(|geometry| (matrix, geometry))
            })
            .collect()
    }

    #[test]
    fn imported_keys_use_six_consistent_row_baselines() {
        let keys = imported_geometry();
        assert_eq!(keys.len(), 81);
        let identities: std::collections::BTreeSet<_> =
            keys.iter().map(|(matrix, _)| *matrix).collect();
        assert_eq!(identities.len(), 81);
        assert!(!identities.contains(&84)); // Zero-area wheel control, not a cap.
        assert!(identities.contains(&0));
        assert!(identities.contains(&81));
        assert!(identities.contains(&89));
        let expected = [27.0, 83.0, 123.0, 163.0, 203.0, 243.0];
        for (_, geometry) in &keys {
            assert!(expected.contains(&geometry.rect.top()));
            assert_eq!(
                geometry.rect.height(),
                if geometry.iso_enter { 74.0 } else { 34.0 }
            );
        }
        for baseline in expected {
            assert!(
                keys.iter()
                    .any(|(_, geometry)| geometry.rect.top() == baseline)
            );
        }
    }

    #[test]
    fn number_row_has_uniform_key_pitch_and_gaps_including_backspace() {
        let keys = imported_geometry();
        let row = [1, 7, 13, 19, 25, 31, 37, 43, 49, 55, 61, 67, 73, 79];
        let geometry = |matrix| keys.iter().find(|(id, _)| *id == matrix).unwrap().1;
        for pair in row.windows(2) {
            let left = geometry(pair[0]).rect;
            let right = geometry(pair[1]).rect;
            assert_eq!(right.left() - left.right(), 6.0);
            assert_eq!(right.top(), left.top());
            if pair[1] != 79 {
                assert_eq!(right.center().x - left.center().x, 40.0);
            }
        }
        assert_eq!(geometry(79).rect.width(), 74.0);
    }

    #[test]
    fn iso_enter_fills_both_rows_and_excludes_the_adjacent_key_notch() {
        let enter = from_oem(566.0, 127.0, 46.0, 28.0, 81).unwrap();
        assert!(enter.contains(Pos2::new(565.0, 140.0)));
        assert!(enter.contains(Pos2::new(595.0, 176.0)));
        assert!(!enter.contains(Pos2::new(565.0, 180.0)));
        assert!(!enter.contains(Pos2::new(620.0, 176.0)));
        assert_eq!(enter.legend_center(), Pos2::new(595.0, 176.0));
        assert_eq!(enter.outline(0.0).len(), 6);
        let slash = from_oem(536.0, 167.0, 26.0, 28.0, 75).unwrap();
        assert_eq!(573.0 - slash.rect.right(), 6.0);
    }

    #[test]
    fn all_imported_faces_are_disjoint_even_beside_the_iso_enter() {
        let keys = imported_geometry();
        for (index, (matrix, a)) in keys.iter().enumerate() {
            for (other_matrix, b) in keys.iter().skip(index + 1) {
                let overlap = a.rect.intersect(b.rect);
                if overlap.width() <= 0.0 || overlap.height() <= 0.0 {
                    continue;
                }
                // Rectangular bounds can overlap Enter's absent notch. Check
                // every pixel centre in the intersection against the true faces.
                for x in (overlap.left().floor() as i32)..(overlap.right().ceil() as i32) {
                    for y in (overlap.top().floor() as i32)..(overlap.bottom().ceil() as i32) {
                        let point = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                        assert!(
                            !(a.contains(point) && b.contains(point)),
                            "matrix {matrix} overlaps matrix {other_matrix} at {point:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_and_zero_area_oem_controls_have_no_visual_face() {
        assert!(from_oem(0.0, 0.0, 0.0, 0.0, 84).is_none());
        assert!(from_oem(f32::NAN, 88.0, 26.0, 28.0, 1).is_none());
        assert!(from_oem(27.0, 88.0, -1.0, 28.0, 1).is_none());
    }
}
