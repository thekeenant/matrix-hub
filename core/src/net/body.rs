//! Collecting an HTTP response body safely: bounded in size, reserved up
//! front when the length is known, and checked for truncation.

use anyhow::{Result, anyhow, bail};

/// Reserved when the server doesn't send a Content-Length
const DEFAULT_RESERVE: usize = 64 * 1024;

#[derive(Debug)]
pub struct BodyCollector {
    body: Vec<u8>,
    expected_len: Option<usize>,
    max_len: usize,
}

impl BodyCollector {
    /// `content_length` is the response's Content-Length header, if any.
    /// Fails if it already exceeds `max_len` or the memory can't be reserved.
    pub fn new(content_length: Option<u64>, max_len: usize) -> Result<Self> {
        let expected_len = content_length.map(usize::try_from).transpose()?;
        if let Some(len) = expected_len.filter(|&len| len > max_len) {
            bail!("Response too large ({len} bytes, limit {max_len})");
        }
        let mut body = Vec::new();
        reserve(
            &mut body,
            expected_len.unwrap_or(DEFAULT_RESERVE.min(max_len)),
        )?;
        Ok(Self {
            body,
            expected_len,
            max_len,
        })
    }

    pub fn len(&self) -> usize {
        self.body.len()
    }

    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// Append the next chunk of the body.
    pub fn push(&mut self, chunk: &[u8]) -> Result<()> {
        if self.body.len() + chunk.len() > self.max_len {
            bail!("Response exceeded {} bytes", self.max_len);
        }
        reserve(&mut self.body, chunk.len())?;
        self.body.extend_from_slice(chunk);
        Ok(())
    }

    /// The complete body; fails if it's shorter or longer than announced.
    pub fn finish(self) -> Result<Vec<u8>> {
        match self.expected_len {
            Some(expected) if expected != self.body.len() => bail!(
                "Truncated response: got {} of {expected} bytes",
                self.body.len()
            ),
            _ => Ok(self.body),
        }
    }
}

/// Fallible allocation: an oversized response must not abort the device.
fn reserve(body: &mut Vec<u8>, additional: usize) -> Result<()> {
    body.try_reserve(additional)
        .map_err(|e| anyhow!("Out of memory for response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    #[gtest]
    fn collects_a_body_of_the_announced_length() -> googletest::Result<()> {
        let mut body = BodyCollector::new(Some(5), 100).or_fail()?;
        expect_true!(body.is_empty());
        body.push(b"hel").or_fail()?;
        body.push(b"lo").or_fail()?;
        expect_that!(body.len(), eq(5));
        expect_that!(body.finish(), ok(eq(&b"hello".to_vec())));
        Ok(())
    }

    #[gtest]
    fn collects_a_body_without_content_length() -> googletest::Result<()> {
        let mut body = BodyCollector::new(None, 100).or_fail()?;
        body.push(b"abc").or_fail()?;
        expect_that!(body.finish(), ok(eq(&b"abc".to_vec())));
        Ok(())
    }

    #[gtest]
    fn rejects_announced_bodies_over_the_limit() {
        expect_that!(
            BodyCollector::new(Some(101), 100),
            err(displays_as(contains_substring("too large")))
        );
    }

    #[gtest]
    fn rejects_bodies_that_grow_past_the_limit() -> googletest::Result<()> {
        let mut body = BodyCollector::new(None, 4).or_fail()?;
        body.push(b"abcd").or_fail()?;
        expect_that!(
            body.push(b"e"),
            err(displays_as(contains_substring("exceeded 4")))
        );
        Ok(())
    }

    #[gtest]
    fn detects_truncated_bodies() -> googletest::Result<()> {
        let mut body = BodyCollector::new(Some(10), 100).or_fail()?;
        body.push(b"short").or_fail()?;
        expect_that!(
            body.finish(),
            err(displays_as(eq("Truncated response: got 5 of 10 bytes")))
        );
        Ok(())
    }

    #[gtest]
    fn reports_allocation_failures() {
        expect_that!(
            BodyCollector::new(None, usize::MAX),
            ok(anything()),
            "a huge limit alone doesn't allocate"
        );
        expect_that!(
            BodyCollector::new(Some(u64::MAX / 2), usize::MAX),
            err(displays_as(contains_substring("Out of memory")))
        );
    }
}
