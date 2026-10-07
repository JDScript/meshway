use super::*;

struct Group {
    sender: Arc<AsyncMutex<Sender>>,
    scope: Scope,
    batch_scope: meshway_transport::ChannelScope,
    fact: LinkFact,
    frames: Vec<PendingFrame>,
}

impl Manager {
    /// Capture actual links synchronously and group at most 32 ready business
    /// frames by frozen channel scope. Selection is never repeated after a wait.
    /// Every group retains all original guards; controls use send_with_receipt.
    pub fn send_batch(
        &self,
        frames: Vec<PendingFrame>,
    ) -> impl std::future::Future<Output = Result<Vec<SendReceipt>, Error>> + Send + 'static + use<>
    {
        let selected = (|| {
            if frames.is_empty() || frames.len() > 32 {
                return Err(Error::Capacity);
            }
            self.check_view()?;
            let links = self.shared.links.lock().map_err(|_| Error::Worker)?;
            let mut groups = BTreeMap::<[u8; 32], Group>::new();
            for frame in frames {
                if !frame.is_data() {
                    return Err(Error::Configuration);
                }
                frame.ensure_current().map_err(|_| Error::Authority)?;
                let link = links.get(&frame.next()).ok_or(Error::Unavailable)?;
                if &link.fact.carrier != frame.carrier() {
                    return Err(Error::Unavailable);
                }
                link.scope.check()?;
                groups
                    .entry(link.fact.id)
                    .or_insert_with(|| Group {
                        sender: link.sender.clone(),
                        scope: link.scope.clone(),
                        batch_scope: link.batch_scope.clone(),
                        fact: link.fact.clone(),
                        frames: Vec::new(),
                    })
                    .frames
                    .push(frame);
            }
            Ok(groups)
        })();
        async move {
            let groups = selected?;
            let sends = groups.into_values().map(|group| async move {
                let receipt = meshway_dataplane::carrier::send_batch_shared(
                    &group.sender,
                    group.frames,
                    group.fact.remote,
                    group.batch_scope,
                    Some(Arc::new(group.scope)),
                )
                .await?;
                Ok(SendReceipt {
                    link: group.fact,
                    admitted_at: receipt.admitted_at,
                })
            });
            futures_util::future::join_all(sends)
                .await
                .into_iter()
                .collect()
        }
    }
}
