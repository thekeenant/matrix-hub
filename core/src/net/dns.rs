//! Captive-portal DNS: every A query is answered with the access point's own
//! IP, so devices joining the setup network land on the config page.

const HEADER_LEN: usize = 12;
const TYPE_A: u16 = 1;
const TYPE_ANY: u16 = 255;

/// Builds the reply to a standard single-question query: the header and
/// question echoed back, plus one A record pointing at the AP for A/ANY
/// queries (other types, e.g. AAAA, get an empty NOERROR answer).
///
/// Anything after the question (EDNS OPT records etc.) is dropped, and
/// NSCOUNT/ARCOUNT are zeroed to match.
pub fn build_response(query: &[u8], ap_ip: [u8; 4]) -> Option<Vec<u8>> {
    if query.len() < HEADER_LEN {
        return None;
    }
    let flags_hi = query[2];
    let is_response = flags_hi & 0x80 != 0;
    let opcode = (flags_hi >> 3) & 0x0F;
    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if is_response || opcode != 0 || qdcount != 1 {
        return None;
    }

    // Walk QNAME's labels to find where the question ends
    let mut pos = HEADER_LEN;
    loop {
        let len = *query.get(pos)? as usize;
        if len == 0 {
            pos += 1;
            break;
        }
        if len & 0xC0 != 0 {
            return None; // compression pointers aren't valid in a question
        }
        pos += 1 + len;
    }
    let qtype = u16::from_be_bytes([*query.get(pos)?, *query.get(pos + 1)?]);
    let question_end = pos + 4; // QTYPE + QCLASS
    if question_end > query.len() {
        return None;
    }

    let answer = matches!(qtype, TYPE_A | TYPE_ANY);

    let mut response = Vec::with_capacity(question_end + 16);
    response.extend_from_slice(&query[..question_end]);
    // QR=1, AA=1, keep RD from the query; RA=0, RCODE=0 (no error)
    response[2] = 0x84 | (flags_hi & 0x01);
    response[3] = 0x00;
    // QDCOUNT=1, ANCOUNT, NSCOUNT=0, ARCOUNT=0
    response[4..12].copy_from_slice(&[0, 1, 0, u8::from(answer), 0, 0, 0, 0]);

    if answer {
        response.extend_from_slice(&[
            0xC0, 0x0C, // name: pointer to the question's QNAME
            0x00, 0x01, // type A
            0x00, 0x01, // class IN
            0x00, 0x00, 0x00, 0x3C, // TTL 60s
            0x00, 0x04, // RDLENGTH
        ]);
        response.extend_from_slice(&ap_ip);
    }

    Some(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use googletest::prelude::*;

    const AP: [u8; 4] = [192, 168, 71, 1];
    const TYPE_AAAA: u16 = 28;

    /// A standard recursive query for `name`, optionally with an EDNS OPT
    /// record in the additional section (as modern resolvers send).
    fn query(id: u16, name: &str, qtype: u16, edns: bool) -> Vec<u8> {
        let mut q = id.to_be_bytes().to_vec();
        q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, u8::from(edns)]);
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&[0, 1]); // class IN
        if edns {
            q.extend_from_slice(&[0, 0, 41, 0x10, 0, 0, 0, 0, 0, 0, 0]);
        }
        q
    }

    #[gtest]
    fn answers_a_query_with_the_access_point_address() {
        let q = query(0xBEEF, "connectivitycheck.gstatic.com", TYPE_A, false);
        let r = build_response(&q, AP).unwrap();

        expect_that!(r[..2], eq(&[0xBE, 0xEF]), "keeps the query ID");
        expect_that!(r[2..4], eq(&[0x85, 0x00]), "QR, AA, RD set; no error");
        expect_that!(
            r[4..12],
            eq(&[0, 1, 0, 1, 0, 0, 0, 0]),
            "1 question, 1 answer"
        );
        expect_that!(r[12..q.len()], eq(&q[12..]), "echoes the question");
        expect_that!(
            r[q.len()..],
            eq(&[0xC0, 0x0C, 0, 1, 0, 1, 0, 0, 0, 0x3C, 0, 4, 192, 168, 71, 1]),
            "A record, TTL 60s, pointing at the AP"
        );
    }

    #[gtest]
    fn answers_any_queries_too() {
        let r = build_response(&query(1, "a.b", TYPE_ANY, false), AP).unwrap();
        expect_that!(r[6..8], eq(&[0, 1]));
    }

    #[gtest]
    fn drops_edns_records_and_zeroes_the_other_counts() {
        let q = query(7, "example.com", TYPE_A, true);
        let question_end = q.len() - 11; // minus the OPT record
        let r = build_response(&q, AP).unwrap();

        expect_that!(r[4..12], eq(&[0, 1, 0, 1, 0, 0, 0, 0]));
        expect_that!(r.len(), eq(question_end + 16));
        expect_that!(r[r.len() - 4..], eq(&AP));
    }

    #[gtest]
    fn gives_other_record_types_an_empty_answer() {
        let q = query(9, "example.com", TYPE_AAAA, false);
        let r = build_response(&q, AP).unwrap();

        expect_that!(r[4..12], eq(&[0, 1, 0, 0, 0, 0, 0, 0]));
        expect_that!(r.len(), eq(q.len()));
    }

    #[gtest]
    fn copies_only_the_rd_flag_from_the_query() {
        let mut q = query(1, "a.b", TYPE_A, false);
        q[2] = 0x00; // RD off
        expect_that!(build_response(&q, AP).unwrap()[2], eq(0x84));
    }

    #[gtest]
    fn ignores_packets_that_are_not_standard_single_queries() {
        let mut response = query(1, "a.b", TYPE_A, false);
        response[2] |= 0x80;
        let mut inverse_query = query(1, "a.b", TYPE_A, false);
        inverse_query[2] |= 1 << 3;
        let mut two_questions = query(1, "a.b", TYPE_A, false);
        two_questions[5] = 2;

        expect_that!(build_response(&response, AP), none());
        expect_that!(build_response(&inverse_query, AP), none());
        expect_that!(build_response(&two_questions, AP), none());
    }

    #[gtest]
    fn ignores_malformed_packets() {
        let mut truncated_name = query(1, "example.com", TYPE_A, false);
        truncated_name.truncate(15);
        let mut truncated_type = query(1, "a.b", TYPE_A, false);
        truncated_type.truncate(truncated_type.len() - 3);
        let mut truncated_class = query(1, "a.b", TYPE_A, false);
        truncated_class.truncate(truncated_class.len() - 1);
        let mut compressed = query(1, "a.b", TYPE_A, false);
        compressed[12] = 0xC0;

        expect_that!(
            build_response(&[0; 11], AP),
            none(),
            "shorter than a header"
        );
        expect_that!(build_response(&truncated_name, AP), none());
        expect_that!(build_response(&truncated_type, AP), none());
        expect_that!(build_response(&truncated_class, AP), none());
        expect_that!(
            build_response(&compressed, AP),
            none(),
            "pointer in question"
        );
    }
}
