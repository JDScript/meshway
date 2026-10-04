use crate::{
    Control, Error,
    identity::{self, Keys},
    random_id,
    storage::Enrollment,
};
use meshway_control_client::{
    CancelEnrollmentRequestPath, QueryEnrollmentRequestPath, RedeemEnrollmentRequestPath,
};
use meshway_model::EnrollmentRequestId;
use meshway_protocol::{
    CreateEnrollmentRequest, EnrollmentDeviceProof, EnrollmentIntent, EnrollmentRequestView,
    EnrollmentResponse, RedeemEnrollmentRequest, WireSecret,
    enrollment::{EnrollmentPhase, enrollment_transcript},
    node::request_digest,
};

impl Control {
    /// The intent and both independent keys are durably written before create.
    /// Retrying after a lost response/restart reuses that exact signed intent.
    pub async fn begin_enrollment(
        &mut self,
        hostname: &str,
    ) -> Result<EnrollmentRequestView, Error> {
        self.ready()?;
        if self.record.binding.is_some() {
            return Err(Error::Lifecycle);
        }
        let hostname = hostname.to_ascii_lowercase();
        if !meshway_configuration::valid_node_hostname(&hostname) {
            return Err(Error::Lifecycle);
        }
        if let Some(pending) = &self.record.enrollment {
            if pending.intent.hostname != hostname {
                return Err(Error::Pending);
            }
        } else {
            let keys = Keys::generate()?;
            let verifier = identity::random_secret()?;
            let intent = EnrollmentIntent {
                id: EnrollmentRequestId::new(random_id()?).map_err(|_| Error::Cryptography)?,
                control_key: keys.control()?,
                dataplane_key: keys.dataplane()?,
                code_challenge: identity::challenge(&verifier),
                hostname,
                existing: None,
            };
            self.record.keys = Some(keys);
            self.record.enrollment = Some(Enrollment {
                intent,
                verifier,
                code: None,
                approved_network: None,
            });
            self.persist().await?;
        }
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let request = CreateEnrollmentRequest {
            intent: pending.intent.clone(),
            signature: self.device_signature("create")?,
        };
        let view = self.client.create_enrollment_request(&request).await?;
        self.check_enrollment(&view)?;
        Ok(view)
    }

    pub async fn poll_enrollment(&mut self) -> Result<EnrollmentRequestView, Error> {
        self.ready()?;
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let response = self
            .client
            .query_enrollment_request(
                &QueryEnrollmentRequestPath {
                    enrollment_request_id: pending.intent.id,
                },
                &self.device_proof("poll")?,
            )
            .await?;
        self.check_enrollment(&response.request)?;
        if let Some(code) = response.code {
            if response.request.phase != EnrollmentPhase::Approved {
                return Err(Error::Contract);
            }
            let approval = response.approval.ok_or(Error::Contract)?;
            let pending = self.record.enrollment.as_mut().ok_or(Error::Lifecycle)?;
            if pending
                .approved_network
                .is_some_and(|network| network != approval.network_id)
            {
                return Err(Error::Contract);
            }
            if pending
                .code
                .as_ref()
                .is_none_or(|old| old.expose() != code.expose())
            {
                pending.code = Some(code);
                pending.approved_network = Some(approval.network_id);
                self.persist().await?;
            }
        }
        Ok(response.request)
    }

    /// The redemption code is already durable. A response lost after Server
    /// commit can be recovered by retrying this same proof/code after restart.
    pub async fn redeem_enrollment(&mut self) -> Result<EnrollmentResponse, Error> {
        self.ready()?;
        if let Some(binding) = &self.record.binding {
            return Ok(binding.clone());
        }
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let code = pending.code.as_ref().ok_or(Error::Lifecycle)?;
        let expected_network = pending.approved_network.ok_or(Error::Record)?;
        let response = self
            .client
            .redeem_enrollment_request(
                &RedeemEnrollmentRequestPath {
                    enrollment_request_id: pending.intent.id,
                },
                &RedeemEnrollmentRequest {
                    proof: self.device_proof("redeem")?,
                    code: WireSecret::new(code.expose().into()),
                },
            )
            .await?;
        if response.network_id != expected_network
            || !self
                .record
                .keys
                .as_ref()
                .ok_or(Error::Record)?
                .matches(&response)?
        {
            return Err(Error::Contract);
        }
        self.record.binding = Some(response.clone());
        self.record.enrollment = None;
        self.persist().await?;
        Ok(response)
    }

    pub async fn cancel_enrollment(&mut self) -> Result<(), Error> {
        self.ready()?;
        let view = self.poll_enrollment().await?;
        if view.phase == EnrollmentPhase::Consumed {
            return Err(Error::Lifecycle);
        }
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        if matches!(
            view.phase,
            EnrollmentPhase::Pending | EnrollmentPhase::Approved
        ) {
            self.client
                .cancel_enrollment_request(
                    &CancelEnrollmentRequestPath {
                        enrollment_request_id: pending.intent.id,
                    },
                    &self.device_proof("cancel")?,
                )
                .await?;
        }
        self.record.enrollment = None;
        self.record.keys = None;
        self.persist().await
    }

    fn device_signature(&self, operation: &str) -> Result<WireSecret, Error> {
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let digest = request_digest(&pending.intent).map_err(|_| Error::Record)?;
        self.record
            .keys
            .as_ref()
            .ok_or(Error::Record)?
            .sign(&enrollment_transcript(pending.intent.id, digest, operation))
    }
    fn device_proof(&self, operation: &str) -> Result<EnrollmentDeviceProof, Error> {
        Ok(EnrollmentDeviceProof {
            signature: self.device_signature(operation)?,
            verifier: WireSecret::new(
                self.record
                    .enrollment
                    .as_ref()
                    .ok_or(Error::Lifecycle)?
                    .verifier
                    .expose()
                    .into(),
            ),
        })
    }
    fn check_enrollment(&self, view: &EnrollmentRequestView) -> Result<(), Error> {
        let intent = &self
            .record
            .enrollment
            .as_ref()
            .ok_or(Error::Lifecycle)?
            .intent;
        if view.id != intent.id
            || view.control_key != intent.control_key
            || view.hostname != intent.hostname
            || view.existing != intent.existing
        {
            return Err(Error::Contract);
        }
        Ok(())
    }
}
