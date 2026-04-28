use crate::PinturappUi;
use crate::io::mesh_loader::MeshData;
use crate::renderer::{BrushDispatch, BrushInput, UvCoverageCache, paint_projected_brush_into};
use crate::{PaintWorkerCommand, PaintWorkerEvent};
use image::RgbaImage;
use std::sync::mpsc;
use std::time::{Duration, Instant};

impl PinturappUi {
    pub(crate) fn clear_history(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
    }

    fn ensure_albedo_texture(&mut self) {
        if self.albedo_texture.is_none() {
            self.albedo_texture = Some(RgbaImage::from_pixel(1024, 1024, image::Rgba([200, 200, 200, 255])));
        }
    }

    pub(crate) fn begin_paint_stroke(&mut self) {
        self.ensure_albedo_texture();
        self.abort_paint_worker();
        self.last_paint_sample_screen_pos = None;
        if let Some(texture) = &self.albedo_texture {
            self.undo_stack.push_back(texture.clone());
            if self.undo_stack.len() > Self::MAX_HISTORY {
                self.undo_stack.pop_front();
            }
            self.redo_stack.clear();
            self.is_dirty = true;
        }
    }

    pub(crate) fn end_paint_stroke(&mut self) {
        self.is_painting_stroke = false;
        self.last_paint_sample_screen_pos = None;
        if let Some(tx) = &self.paint_worker_tx {
            let _ = tx.send(PaintWorkerCommand::Finish);
        }
    }

    pub(crate) fn undo_paint(&mut self) {
        self.abort_paint_worker();
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(previous) = self.undo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.redo_stack.push_back(current);
        if self.redo_stack.len() > Self::MAX_HISTORY {
            self.redo_stack.pop_front();
        }
        self.albedo_texture = Some(previous);
        self.is_dirty = true;
        self.viewport_needs_refresh = true;
    }

    pub(crate) fn redo_paint(&mut self) {
        self.abort_paint_worker();
        let Some(current) = self.albedo_texture.take() else {
            return;
        };
        let Some(next) = self.redo_stack.pop_back() else {
            self.albedo_texture = Some(current);
            return;
        };
        self.undo_stack.push_back(current);
        if self.undo_stack.len() > Self::MAX_HISTORY {
            self.undo_stack.pop_front();
        }
        self.albedo_texture = Some(next);
        self.is_dirty = true;
        self.viewport_needs_refresh = true;
    }

    pub(crate) fn abort_paint_worker(&mut self) {
        if let Some(tx) = &self.paint_worker_tx {
            let _ = tx.send(PaintWorkerCommand::Abort);
        }
        self.paint_worker_tx = None;
        self.paint_worker_rx = None;
        if let Some(join) = self.paint_worker_join.take() {
            let _ = join.join();
        }
    }

    pub(crate) fn poll_paint_worker(&mut self) {
        let mut finalized_texture = None;
        if let Some(rx) = &self.paint_worker_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    PaintWorkerEvent::Preview(texture) => {
                        self.albedo_texture = Some(texture);
                        self.viewport_needs_refresh = true;
                        self.is_dirty = true;
                    }
                    PaintWorkerEvent::Finished(texture) => {
                        finalized_texture = Some(texture);
                    }
                }
            }
        }

        if let Some(texture) = finalized_texture {
            self.albedo_texture = Some(texture);
            self.viewport_needs_refresh = true;
            self.is_dirty = true;
            self.paint_worker_tx = None;
            self.paint_worker_rx = None;
            if let Some(join) = self.paint_worker_join.take() {
                let _ = join.join();
            }
        }
    }

    fn ensure_paint_worker(&mut self, mesh: &MeshData) -> bool {
        if self.paint_worker_tx.is_some() {
            return true;
        }

        let Some(texture) = self.albedo_texture.clone() else {
            return false;
        };
        let mesh = mesh.clone();
        let config = self.paint_pipeline_config.clone();
        let (tx, rx) = mpsc::channel::<PaintWorkerCommand>();
        let (event_tx, event_rx) = mpsc::channel::<PaintWorkerEvent>();
        let join = std::thread::spawn(move || {
            let mut texture = texture;
            let mut coverage_cache = UvCoverageCache::default();
            let mut pending_stamps: Vec<(BrushInput, BrushDispatch)> = Vec::new();
            let mut gpu_session: Option<crate::renderer::gpu_paint::GpuPaintSession> = None;
            let mut painted_since_preview = false;
            let mut last_preview = Instant::now();
            let preview_interval = Duration::from_millis(90);

            loop {
                let command = match rx.recv() {
                    Ok(command) => command,
                    Err(_) => break,
                };

                let mut pending_finish = false;
                match command {
                    PaintWorkerCommand::Stamp { input, dispatch } => {
                        pending_stamps.push((input, dispatch));
                    }
                    PaintWorkerCommand::Finish => pending_finish = true,
                    PaintWorkerCommand::Abort => break,
                }

                while !pending_finish {
                    let next = match rx.try_recv() {
                        Ok(next) => next,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => {
                            pending_finish = true;
                            break;
                        }
                    };
                    match next {
                        PaintWorkerCommand::Stamp { input, dispatch } => {
                            pending_stamps.push((input, dispatch));
                        }
                        PaintWorkerCommand::Finish => pending_finish = true,
                        PaintWorkerCommand::Abort => return,
                    }
                }

                if !pending_stamps.is_empty() {
                    let mut batch_painted = false;
                    if config.use_gpu_compute_experimental {
                        if gpu_session.is_none() {
                            gpu_session =
                                crate::renderer::gpu_paint::GpuPaintSession::new(&texture, &mut coverage_cache, &mesh);
                        }
                    } else {
                        gpu_session = None;
                    }

                    if let Some(session) = gpu_session.as_mut() {
                        batch_painted = session.apply_stamps(&pending_stamps);
                    } else {
                        for (input, dispatch) in &pending_stamps {
                            if paint_projected_brush_into(
                                &mut texture,
                                &mesh,
                                *input,
                                *dispatch,
                                Some(&mut coverage_cache),
                                &config,
                            ) {
                                batch_painted = true;
                            }
                        }
                    }
                    pending_stamps.clear();
                    painted_since_preview |= batch_painted;
                }

                if painted_since_preview && last_preview.elapsed() >= preview_interval {
                    if let Some(session) = gpu_session.as_mut() {
                        let _ = session.readback_if_dirty(&mut texture);
                    }
                    if event_tx.send(PaintWorkerEvent::Preview(texture.clone())).is_err() {
                        return;
                    }
                    painted_since_preview = false;
                    last_preview = Instant::now();
                }

                if pending_finish {
                    if let Some(session) = gpu_session.as_mut() {
                        let _ = session.readback_if_dirty(&mut texture);
                    }
                    let _ = event_tx.send(PaintWorkerEvent::Finished(texture));
                    return;
                }
            }
        });

        self.paint_worker_tx = Some(tx);
        self.paint_worker_rx = Some(event_rx);
        self.paint_worker_join = Some(join);
        true
    }

    pub(crate) fn paint_projected_brush(
        &mut self,
        mesh: &MeshData,
        input: BrushInput,
        dispatch: BrushDispatch,
    ) {
        self.ensure_albedo_texture();
        if !self.ensure_paint_worker(mesh) {
            return;
        }
        if let Some(tx) = &self.paint_worker_tx {
            if tx.send(PaintWorkerCommand::Stamp { input, dispatch }).is_ok() {
                self.is_dirty = true;
                return;
            }
        }

        // Fallback path if worker is unavailable.
        self.abort_paint_worker();
        let Some(texture) = self.albedo_texture.as_mut() else {
            return;
        };
        if paint_projected_brush_into(
            texture,
            mesh,
            input,
            dispatch,
            Some(self.uv_coverage_cache.get_or_insert_with(Default::default)),
            &self.paint_pipeline_config,
        ) {
            self.is_dirty = true;
            self.viewport_needs_refresh = true;
        }
    }
}
