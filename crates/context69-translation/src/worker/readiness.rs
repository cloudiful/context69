use anyhow::Result;
use uuid::Uuid;

use super::TranslationService;

pub(super) enum ProcessingPermit {
    Ready,
    Probe(Uuid),
    Blocked,
}

impl TranslationService {
    pub(super) async fn acquire_processing_permit(&self) -> Result<ProcessingPermit> {
        if self.readiness.is_ready().await.unwrap_or(false) {
            return Ok(ProcessingPermit::Ready);
        }

        Ok(match self.readiness.reserve_probe().await? {
            Some(token) => ProcessingPermit::Probe(token),
            None => ProcessingPermit::Blocked,
        })
    }

    pub(super) async fn processing_ready_for(&self, probe_token: Option<Uuid>) -> bool {
        match probe_token {
            Some(token) => self
                .readiness
                .is_ready_for_probe(token)
                .await
                .unwrap_or(false),
            None => self.readiness.is_ready().await.unwrap_or(false),
        }
    }

    pub(super) async fn finish_processing_probe(
        &self,
        probe_token: Uuid,
        result: &Result<bool>,
    ) -> Result<()> {
        if matches!(result, Ok(true)) {
            self.readiness.complete_probe(probe_token).await
        } else {
            self.readiness.abandon_probe(probe_token).await
        }
    }
}
