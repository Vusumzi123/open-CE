use glam::Vec3;

pub struct MovementSample {
    pub frame: u64,
    pub dt: f32,
    pub position: Vec3,
    pub eye: Vec3,
    pub velocity_y: f32,
    pub on_ground: bool,
    pub yaw: f32,
    pub pitch: f32,
    pub direction: Vec3,
    pub running: bool,
    pub jump: bool,
}

impl MovementSample {
    pub fn line(&self) -> String {
        let fps = if self.dt > 0.0 { 1.0 / self.dt } else { 0.0 };
        format!(
            "debug: frame={} dt={:.4} fps={:.1} pos=({:.2},{:.2},{:.2}) eye=({:.2},{:.2},{:.2}) vel_y={:.2} grounded={} yaw={:.3} pitch={:.3} dir=({:.2},{:.2},{:.2}) run={} jump={}",
            self.frame,
            self.dt,
            fps,
            self.position.x,
            self.position.y,
            self.position.z,
            self.eye.x,
            self.eye.y,
            self.eye.z,
            self.velocity_y,
            self.on_ground,
            self.yaw,
            self.pitch,
            self.direction.x,
            self.direction.y,
            self.direction.z,
            self.running,
            self.jump,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn movement_line_reports_frame_and_motion() {
        let line = MovementSample {
            frame: 7,
            dt: 0.02,
            position: Vec3::new(1.0, 2.0, 3.0),
            eye: Vec3::new(1.0, 122.0, 3.0),
            velocity_y: -4.0,
            on_ground: false,
            yaw: 0.5,
            pitch: -0.25,
            direction: Vec3::new(1.0, 0.0, 0.0),
            running: true,
            jump: false,
        }
        .line();

        assert!(line.starts_with("debug: frame=7 "));
        assert!(line.contains("fps=50.0"));
        assert!(line.contains("vel_y=-4.00"));
        assert!(line.contains("grounded=false"));
        assert!(line.contains("run=true"));
    }
}
