//! Attention Kernel：所有注意力请求必须先经过确定性裁决，才能进入用户表面。

use chrono::{DateTime, Duration, Utc};
use else_protocol::attention::{AttentionRequest, ExpectedLoss, Ruling};
use else_protocol::intent::AttentionPolicy;

#[derive(Debug, Default, Clone, Copy)]
pub struct AttentionKernel;

impl AttentionKernel {
    /// 规则优先级：用户禁止打断 > 已到最迟时间 > 预期损失与可逆性。
    /// 返回值必须由调用方持久化为 AttentionRuled，不能只留在进程内。
    pub fn rule(
        &self,
        request: &AttentionRequest,
        policy: &AttentionPolicy,
        now: DateTime<Utc>,
    ) -> Ruling {
        if request.decide_by.is_some_and(|deadline| deadline <= now) {
            return Ruling::Interrupt;
        }
        if !policy.allow_interrupt {
            return Ruling::Batch;
        }
        match request.expected_loss {
            ExpectedLoss::Critical => Ruling::Interrupt,
            ExpectedLoss::High if request.blocking => Ruling::Interrupt,
            ExpectedLoss::High => Ruling::Batch,
            ExpectedLoss::Medium if request.blocking && !request.reversible => Ruling::Interrupt,
            ExpectedLoss::Medium if request.blocking => Ruling::Defer {
                until: defer_until(request, policy, now),
            },
            ExpectedLoss::Medium => Ruling::Batch,
            ExpectedLoss::Low if request.blocking => Ruling::Batch,
            ExpectedLoss::Low if request.reversible => Ruling::Silent,
            ExpectedLoss::Low => Ruling::Batch,
        }
    }
}

fn defer_until(
    request: &AttentionRequest,
    policy: &AttentionPolicy,
    now: DateTime<Utc>,
) -> DateTime<Utc> {
    let batch_at = now + Duration::minutes(i64::from(policy.batch_interval_minutes.max(1)));
    request
        .decide_by
        .map(|deadline| deadline.min(batch_at))
        .unwrap_or(batch_at)
        .max(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use else_protocol::attention::{AttentionKind, DecisionOption};
    use else_protocol::ids::AttentionId;

    fn request(loss: ExpectedLoss, blocking: bool, reversible: bool) -> AttentionRequest {
        AttentionRequest {
            id: AttentionId::new(),
            kind: AttentionKind::Authorization,
            summary: "test".into(),
            blocked_reason: "test".into(),
            expected_loss: loss,
            blocking,
            reversible,
            decide_by: None,
            options: vec![DecisionOption {
                id: "approve".into(),
                label: "approve".into(),
                recommended: true,
            }],
            default_option: Some("approve".into()),
            related_call_key: None,
        }
    }

    fn policy(allow_interrupt: bool) -> AttentionPolicy {
        AttentionPolicy {
            allow_interrupt,
            batch_interval_minutes: 30,
        }
    }

    #[test]
    fn high_blocking_requests_interrupt_but_policy_can_downgrade() {
        let kernel = AttentionKernel;
        let now = Utc::now();
        let request = request(ExpectedLoss::High, true, true);
        assert_eq!(kernel.rule(&request, &policy(true), now), Ruling::Interrupt);
        assert_eq!(kernel.rule(&request, &policy(false), now), Ruling::Batch);
    }

    #[test]
    fn low_reversible_requests_stay_silent() {
        let ruling = AttentionKernel.rule(
            &request(ExpectedLoss::Low, false, true),
            &policy(true),
            Utc::now(),
        );
        assert_eq!(ruling, Ruling::Silent);
    }

    #[test]
    fn expired_deadline_overrides_interrupt_policy() {
        let mut request = request(ExpectedLoss::Low, false, true);
        request.decide_by = Some(Utc::now() - Duration::minutes(1));
        assert_eq!(
            AttentionKernel.rule(&request, &policy(false), Utc::now()),
            Ruling::Interrupt
        );
    }
}
