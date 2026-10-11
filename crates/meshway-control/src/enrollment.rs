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
    enrollment::{EnrollmentPhase, ExistingNodeBinding, enrollment_transcript},
    node::request_digest,
};

impl Control {
    /// The intent and both independent keys are durably written before create.
    /// Retrying after a lost response/restart reuses that exact signed intent.
    pub async fn begin_enrollment(
        &mut self,
        hostname: &str,
    ) -> Result<EnrollmentRequestView, Error> {
        self.prepare_enrollment(hostname, None).await?;
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        if pending.access_key.is_some() {
            return Err(Error::Pending);
        }
        let request = CreateEnrollmentRequest {
            intent: pending.intent.clone(),
            signature: self.device_signature("create")?,
        };
        let view = self.client.create_enrollment_request(&request).await?;
        self.check_enrollment(&view)?;
        Ok(view)
    }

    /// Enrollment again of the held identity: the same network, Node and
    /// keys, declared as `existing` in a newly signed intent. The device
    /// proves its control key; the Server still requires a Console approval
    /// (enrollment keys cannot re-enroll) and an eligible, unrevoked
    /// identity. Redemption replaces only the binding, which re-pins the
    /// Server's current signing key: the recovery after a missed rotation
    /// window. The held lease is not withdrawn meanwhile.
    pub async fn begin_reenrollment(
        &mut self,
        hostname: &str,
    ) -> Result<EnrollmentRequestView, Error> {
        self.ready()?;
        let binding = self.record.binding.as_ref().ok_or(Error::Lifecycle)?;
        match &self.record.enrollment {
            Some(pending) if pending.intent.existing.is_some() => {}
            Some(_) => return Err(Error::Pending),
            None => {
                if self.record.rotation.is_some() {
                    return Err(Error::Pending);
                }
                let hostname = hostname.to_ascii_lowercase();
                if !meshway_configuration::valid_node_hostname(&hostname) {
                    return Err(Error::Lifecycle);
                }
                let keys = self.record.keys.as_ref().ok_or(Error::Record)?;
                let verifier = identity::random_secret()?;
                let intent = EnrollmentIntent {
                    id: EnrollmentRequestId::new(random_id()?).map_err(|_| Error::Cryptography)?,
                    control_key: keys.control()?,
                    dataplane_key: keys.dataplane()?,
                    code_challenge: identity::challenge(&verifier),
                    hostname,
                    existing: Some(ExistingNodeBinding {
                        network_id: binding.network_id,
                        node_id: binding.node_id,
                        key_id: binding.keys.control.id,
                        key_generation: binding.keys.control.generation,
                    }),
                };
                self.record.enrollment = Some(Enrollment {
                    intent,
                    verifier,
                    code: None,
                    approved_network: None,
                    access_key: None,
                });
                self.persist().await?;
            }
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

    /// A re-enrollment of the held identity is pending.
    pub fn reenrollment_pending(&self) -> bool {
        self.record
            .enrollment
            .as_ref()
            .is_some_and(|pending| pending.intent.existing.is_some())
    }

    async fn prepare_enrollment(
        &mut self,
        hostname: &str,
        access_key: Option<WireSecret>,
    ) -> Result<(), Error> {
        self.ready()?;
        if self.record.binding.is_some() {
            return Err(Error::Lifecycle);
        }
        let hostname = hostname.to_ascii_lowercase();
        if !meshway_configuration::valid_node_hostname(&hostname) {
            return Err(Error::Lifecycle);
        }
        if let Some(pending) = &self.record.enrollment {
            if pending.intent.hostname != hostname
                || pending.access_key.as_ref().map(WireSecret::expose)
                    != access_key.as_ref().map(WireSecret::expose)
            {
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
                access_key,
            });
            self.persist().await?;
        }
        Ok(())
    }

    /// Persist the access key and held-key intent before one-use redemption.
    /// Retry/restart uses the same private request; the key never enters status.
    pub async fn enroll_with_key(
        &mut self,
        hostname: &str,
        key: WireSecret,
    ) -> Result<EnrollmentResponse, Error> {
        self.prepare_enrollment(hostname, Some(key)).await?;
        self.resume_key_enrollment().await
    }

    pub fn key_enrollment_pending(&self) -> bool {
        self.record
            .enrollment
            .as_ref()
            .is_some_and(|pending| pending.access_key.is_some())
    }

    pub async fn resume_key_enrollment(&mut self) -> Result<EnrollmentResponse, Error> {
        self.ready()?;
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let response = self
            .client
            .enroll_node_with_key(&meshway_protocol::KeyEnrollmentRequest {
                enrollment_key: WireSecret::new(
                    pending
                        .access_key
                        .as_ref()
                        .ok_or(Error::Lifecycle)?
                        .expose()
                        .to_owned(),
                ),
                request: CreateEnrollmentRequest {
                    intent: pending.intent.clone(),
                    signature: self.device_signature("create")?,
                },
                verifier: WireSecret::new(pending.verifier.expose().to_owned()),
            })
            .await?;
        if !self
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
        if let Some(binding) = &self.record.binding
            && !self.reenrollment_pending()
        {
            return Ok(binding.clone());
        }
        let pending = self.record.enrollment.as_ref().ok_or(Error::Lifecycle)?;
        let code = pending.code.as_ref().ok_or(Error::Lifecycle)?;
        let existing = pending.intent.existing.clone();
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
            || existing.as_ref().is_some_and(|existing| {
                response.network_id != existing.network_id || response.node_id != existing.node_id
            })
            || !self
                .record
                .keys
                .as_ref()
                .ok_or(Error::Record)?
                .matches(&response)?
        {
            return Err(Error::Contract);
        }
        // A re-enrollment replaces only the binding: the same identity and
        // keys, with the Server signing key current at redemption pinned.
        // The Server retired the previous session and instance.
        if existing.is_some() {
            self.fence();
            self.session = None;
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
        // A cancelled re-enrollment keeps the held identity and its keys.
        if self.record.binding.is_none() {
            self.record.keys = None;
        }
        self.record.enrollment = None;
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
