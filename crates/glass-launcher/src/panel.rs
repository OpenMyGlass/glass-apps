//! A full-screen single card shown over the timeline (setup steps, dictation), rendered
//! through a one-card timeline like the menus' sub-screens, so it uses the same layer cache
//! and renderer.

use glass_gfx::{Compositor, GfxError};
use glass_ui::{Card, CardContent, RenderContext, Timeline};

pub struct Panel {
    screen: Timeline,
}

impl Panel {
    pub fn new(content: CardContent, ctx: RenderContext) -> Panel {
        let mut screen = Timeline::new(vec![], Card::new(0, content), vec![]);
        screen.set_context(ctx);
        Panel { screen }
    }

    /// Shows `content` instead (re-rendered only if it differs).
    pub fn set(&mut self, content: CardContent) {
        // Compared without the clock: only the content matters here.
        const FIXED: RenderContext = RenderContext {
            now_s: 0,
            hour: 0,
            minute: 0,
        };
        let same = glass_ui::card::render_key(self.screen.current_card(), &FIXED)
            == glass_ui::card::render_key(&Card::new(0, content.clone()), &FIXED);
        if !same {
            self.screen.update_card(0, content);
        }
    }

    pub fn content(&self) -> &CardContent {
        &self.screen.current_card().content
    }

    pub fn needs_redraw(&self) -> bool {
        self.screen.needs_redraw()
    }

    pub fn set_context(&mut self, ctx: RenderContext) {
        self.screen.set_context(ctx);
    }

    pub fn render(&mut self, comp: &mut dyn Compositor) -> Result<(), GfxError> {
        self.screen.render(comp)
    }

    pub fn release(&mut self, comp: &mut dyn Compositor) {
        self.screen.release_layers(comp);
    }
}
