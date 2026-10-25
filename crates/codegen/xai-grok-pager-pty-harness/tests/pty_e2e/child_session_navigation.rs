use super::*;

pub(crate) const PARENT_NAVIGATION: &str = "[↑]";

impl SendMessageScenario {
    pub(crate) fn verify_child_session_info(&mut self) {
        self.harness
            .inject_keys(b"/session-info\r")
            .expect("request child session info");
        self.harness
            .wait_for_text("Session info", Duration::from_secs(20))
            .expect("child session info opens");
        self.harness
            .wait_for_text(CHILD_ID, ROW_TIMEOUT)
            .expect("session info identifies the fixture child");
        self.harness
            .inject_keys(keys::ESC)
            .expect("close session info");
        self.harness
            .wait_for_text_absent("Session info", Duration::from_secs(10))
            .expect("session info closes");
    }

    pub(crate) fn return_to_parent(&mut self, parent_text: &str) {
        let (row, col) = locate_screen_text(&self.harness.screen_contents(), PARENT_NAVIGATION)
            .expect("child header exposes its parent navigation button");
        self.harness
            .inject_keys(sgr_mouse(0, row, col, 'M').as_bytes())
            .expect("press parent navigation");
        self.harness
            .inject_keys(sgr_mouse(0, row, col, 'm').as_bytes())
            .expect("release parent navigation");
        self.harness
            .wait_for_text_absent(PARENT_NAVIGATION, ROW_TIMEOUT)
            .expect("fixture root parent is visible");
        self.wait_for_row(parent_text);
    }
}
