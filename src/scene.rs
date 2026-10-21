use eframe::egui::Vec2;

const CAMERA_DISTANCE: f32 = 1500.0;

/// Perspective camera for centred keyboard coordinates. Positive height rises
/// above the plate, and positive depth faces the viewer. Perspective fades with
/// the pitch so the 2D endpoint remains an exact, height-independent plan view.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: -0.20,
            pitch: 0.78,
        }
    }
}
impl Camera {
    pub fn blended(self, amount: f32) -> Self {
        Self {
            yaw: self.yaw * amount,
            pitch: self.pitch * amount,
        }
    }
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw = (self.yaw + delta.x * 0.004).clamp(-0.42, 0.42);
        self.pitch = (self.pitch + delta.y * 0.004).clamp(0.2, 0.95);
    }
    pub fn project(self, point: Vec2) -> Vec2 {
        self.project_height(point, 0.0)
    }
    pub fn project_height(self, point: Vec2, height: f32) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let r = c * point.x - s * point.y;
        let q = s * point.x + c * point.y;
        let depth = q * sin_pitch + height * cos_pitch;
        let factor = CAMERA_DISTANCE / (CAMERA_DISTANCE - depth * sin_pitch);
        Vec2::new(r, q * cos_pitch - height * sin_pitch) * factor
    }
    pub fn unproject(self, point: Vec2) -> Vec2 {
        self.unproject_height(point, 0.0)
    }
    /// Intersect the view ray with a plane at the known face height. This is the
    /// algebraic inverse of project_height, including its perspective scaling.
    pub fn unproject_height(self, point: Vec2, height: f32) -> Vec2 {
        let (s, c) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let constant = CAMERA_DISTANCE - height * cos_pitch * sin_pitch;
        let q = (point.y * constant + CAMERA_DISTANCE * height * sin_pitch)
            / (CAMERA_DISTANCE * cos_pitch + point.y * sin_pitch * sin_pitch);
        let r = point.x * (constant - q * sin_pitch * sin_pitch) / CAMERA_DISTANCE;
        Vec2::new(c * r + s * q, -s * r + c * q)
    }
    pub fn depth(self, point: Vec2, height: f32) -> f32 {
        let q = self.yaw.sin() * point.x + self.yaw.cos() * point.y;
        q * self.pitch.sin() + height * self.pitch.cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hit_coordinates_roundtrip_across_camera_extremes_and_transitions() {
        for yaw in [-0.42, -0.20, 0.0, 0.42] {
            for pitch in [0.2, 0.78, 0.95] {
                for blend in [0.0, 0.25, 0.5, 1.0] {
                    let camera = Camera { yaw, pitch }.blended(blend);
                    for point in [
                        Vec2::new(-345.0, -155.0),
                        Vec2::new(345.0, 155.0),
                        Vec2::ZERO,
                    ] {
                        assert!((camera.unproject(camera.project(point)) - point).length() < 0.001);
                    }
                }
            }
        }
    }
    #[test]
    fn sculpted_face_coordinates_roundtrip_at_every_supported_height() {
        for yaw in [-0.42, -0.20, 0.0, 0.42] {
            for pitch in [0.2, 0.78, 0.95] {
                for blend in [0.0, 0.25, 0.5, 1.0] {
                    let camera = Camera { yaw, pitch }.blended(blend);
                    for height in [-24.0, -6.0, 0.0, 6.0, 12.0, 18.0, 28.0, 48.0] {
                        for point in [
                            Vec2::new(-345.0, -155.0),
                            Vec2::new(345.0, -155.0),
                            Vec2::new(345.0, 155.0),
                            Vec2::new(-345.0, 155.0),
                            Vec2::ZERO,
                        ] {
                            let projected = camera.project_height(point, height);
                            let restored = camera.unproject_height(projected, height);
                            assert!(
                                (restored - point).length() < 0.001,
                                "Roundtrip at {camera:?}, height {height}: {point:?} became {restored:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn perspective_makes_front_keys_larger_and_depth_orders_higher_faces_nearer() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.78,
        };
        let projected_width = |y| {
            (camera.project_height(Vec2::new(20.0, y), 12.0)
                - camera.project_height(Vec2::new(-20.0, y), 12.0))
            .length()
        };
        assert!(projected_width(155.0) > projected_width(-155.0) * 1.08);
        assert!(
            camera.depth(Vec2::new(0.0, 155.0), 0.0) > camera.depth(Vec2::new(0.0, -155.0), 0.0)
        );
        assert!(camera.depth(Vec2::ZERO, 12.0) > camera.depth(Vec2::ZERO, 0.0));
    }
    #[test]
    fn scene_volume_stays_finite_and_bounded_through_orbit_and_view_transitions() {
        for yaw in [-0.42, -0.20, 0.0, 0.42] {
            for pitch in [0.2, 0.78, 0.95] {
                for blend in [0.0, 0.25, 0.5, 1.0] {
                    let camera = Camera { yaw, pitch }.blended(blend);
                    for x in [-360.0, -180.0, 0.0, 180.0, 360.0] {
                        for y in [-175.0, 0.0, 175.0] {
                            for height in [-24.0, 0.0, 12.0, 28.0, 48.0] {
                                let point = Vec2::new(x, y);
                                let projected = camera.project_height(point, height);
                                assert!(projected.x.is_finite() && projected.y.is_finite());
                                assert!(projected.length() < 600.0);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn orbit_stays_in_readable_bounds_and_flat_view_is_exact() {
        let mut camera = Camera::default();
        camera.orbit(Vec2::splat(10000.0));
        assert_eq!(camera.yaw, 0.42);
        assert_eq!(camera.pitch, 0.95);
        let point = Vec2::new(123.0, 45.0);
        assert_eq!(camera.blended(0.0).project(point), point);
        for height in [-24.0, 0.0, 6.0, 12.0, 28.0, 48.0] {
            assert_eq!(camera.blended(0.0).project_height(point, height), point);
            assert_eq!(camera.blended(0.0).unproject_height(point, height), point);
        }
        camera.orbit(Vec2::splat(-10000.0));
        assert_eq!(camera.yaw, -0.42);
        assert_eq!(camera.pitch, 0.2);
    }
}
