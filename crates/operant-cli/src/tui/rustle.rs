// (iter-392: the `#![allow(dead_code)]` suppression is gone — this module is
// reached from `render/welcome.rs::render_welcome_box`.)

// (iter-144: RustlePose enum deleted — rustle_lines() ignores the pose
// anyway (`let _ = pose;`). The App fields (rustle_current_pose,
// rustle_pose_until, rustle_temp_pose, rustle_next_blink) and the
// tick_rustle_pose() method were also deleted — tick was never called.)

#[cfg(test)]
mod tests {}
