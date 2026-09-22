//! Scripted provider for downstream unit and integration tests.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::{DecisionError, DecisionProvider, DecisionRequest, DecisionResponse};

/// A thread-safe provider that returns scripted responses and records requests.
#[derive(Clone, Debug)]
pub struct FakeProvider {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    responses: Mutex<VecDeque<Result<DecisionResponse, DecisionError>>>,
    requests: Mutex<Vec<DecisionRequest>>,
}

impl FakeProvider {
    pub fn new(responses: Vec<Result<DecisionResponse, DecisionError>>) -> Self {
        Self {
            inner: Arc::new(Inner {
                responses: Mutex::new(responses.into()),
                requests: Mutex::new(Vec::new()),
            }),
        }
    }

    pub fn requests(&self) -> Vec<DecisionRequest> {
        self.inner
            .requests
            .lock()
            .expect("fake provider request lock is not poisoned")
            .clone()
    }

    pub fn remaining(&self) -> usize {
        self.inner
            .responses
            .lock()
            .expect("fake provider response lock is not poisoned")
            .len()
    }
}

impl DecisionProvider for FakeProvider {
    fn evaluate(&self, request: &DecisionRequest) -> Result<DecisionResponse, DecisionError> {
        self.inner
            .requests
            .lock()
            .expect("fake provider request lock is not poisoned")
            .push(request.clone());

        let response = self
            .inner
            .responses
            .lock()
            .expect("fake provider response lock is not poisoned")
            .pop_front()
            .ok_or_else(|| DecisionError::Provider {
                message: "fake provider has no response remaining".to_owned(),
            })??;
        response
            .validate_for(request)
            .map_err(DecisionError::invalid_response)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{Answer, NoulAnswer, NoulQuestion, Probability, Question, QuestionId};

    fn fixture() -> (DecisionRequest, DecisionResponse) {
        let id = QuestionId::new("needs_tests").unwrap();
        let request = DecisionRequest::new(
            json!({ "after": "fn changed();" }),
            vec![Question::Noul(
                NoulQuestion::new(id.clone(), "Does this need tests?").unwrap(),
            )],
        )
        .unwrap();
        let response = DecisionResponse::new(
            "fake",
            "fake-v1",
            vec![(
                id,
                Answer::Noul(NoulAnswer::new(Probability::new(0.8).unwrap())),
            )],
            None,
        )
        .unwrap();
        (request, response)
    }

    #[test]
    fn returns_scripted_responses_and_records_requests() {
        let (request, response) = fixture();
        let provider = FakeProvider::new(vec![Ok(response.clone())]);

        assert_eq!(provider.evaluate(&request), Ok(response));
        assert_eq!(provider.requests(), vec![request]);
        assert_eq!(provider.remaining(), 0);
    }

    #[test]
    fn rejects_a_response_that_does_not_match_the_request() {
        let (request, _) = fixture();
        let response = DecisionResponse::new("fake", "fake-v1", vec![], None).unwrap();
        let provider = FakeProvider::new(vec![Ok(response)]);

        assert!(matches!(
            provider.evaluate(&request),
            Err(DecisionError::InvalidResponse { .. })
        ));
    }

    #[test]
    fn reports_exhaustion_after_recording_the_request() {
        let (request, _) = fixture();
        let provider = FakeProvider::new(vec![]);

        assert!(matches!(
            provider.evaluate(&request),
            Err(DecisionError::Provider { .. })
        ));
        assert_eq!(provider.requests(), vec![request]);
    }
}
