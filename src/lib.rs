#![forbid(unsafe_code)]

//! GS1 UK TRADACOMS: an interchange as its segments, one part each, named
//! by the tag — `STX`, `MHD`, `TYP`, `CLO`, `MTR`, `END` — in the order they
//! lie. The delimiters are fixed by the standard and announced by nothing:
//! `'` ends a segment, `+` separates elements, `:` their components, `?`
//! releases, and a tag is followed by `=` rather than the element separator.
//! The announced type is the message type of the first `MHD`, the first
//! component of its second element — `ORDHDR`, `INVFIL`, `ACKHDR` — when
//! there is an `MHD` (ADR-0047).
//!
//! The walk is the Foundation's `message::segment`: this shape brings the
//! delimiters and reads one segment. A contract checks everything else.

use message::segment::{self, Delimiters};
use message::{Shape, ShapeError, Shaped};
use stream::Stream;

/// The TRADACOMS shape.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tradacoms;

/// The delimiters TRADACOMS fixes: `'`, `+`, `:`, `=` after the tag, `?`.
pub const DELIMITERS: Delimiters = Delimiters::new(b'\'', b'+', b':')
    .with_tag(b'=')
    .with_release(b'?');

impl Shape for Tradacoms {
    fn technology(&self) -> &'static str {
        "edi-tradacoms"
    }

    fn media_types(&self) -> &'static [&'static str] {
        &["application/edi-consent"]
    }

    fn recognises(&self, bytes: &[u8]) -> bool {
        bytes.starts_with(b"STX=")
    }

    fn shape(&self, stream: &Stream) -> Result<Shaped, ShapeError> {
        let segments = segment::segments(stream.bytes(), &DELIMITERS)
            .map_err(|stop| ShapeError::refused("edi-tradacoms", stop))?;
        let message_type = segment::first(&segments, "MHD")
            .and_then(|mhd| mhd.component(1, 0, &DELIMITERS))
            .filter(|kind| !kind.is_empty())
            .map(|kind| String::from_utf8_lossy(&DELIMITERS.unreleased(kind)).into_owned());
        let media = stream.media_type().unwrap_or("application/edi-consent");
        Ok(Shaped {
            parts: segment::parts(&segments, media),
            message_type,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::StreamId;

    const ORDERS: &[u8] = b"STX=ANA:1+5000000000000:SENDER+5000000000001:RECEIVER+240101:120000\
+000001++ORDHDR'\nMHD=1+ORDHDR:9'\nTYP=0430+NEW-ORDERS'\nSDT=5000000000000'\n\
CDT=5000000000001:BOB?'S+SHOP'\nMTR=5'\nEND=1'\n";

    fn stream(bytes: &[u8], media: Option<&str>) -> Stream {
        Stream::new(StreamId::new(1), bytes.to_vec(), media.map(str::to_string))
    }

    #[test]
    fn an_interchange_is_its_segments_named_by_tag_and_the_mhd_message_type_is_announced() {
        let shaped = Tradacoms.shape(&stream(ORDERS, None)).expect("well-formed");
        let names: Vec<&str> = shaped
            .parts
            .iter()
            .filter_map(|p| p.name.as_deref())
            .collect();
        assert_eq!(names, ["STX", "MHD", "TYP", "SDT", "CDT", "MTR", "END"]);
        assert_eq!(shaped.parts[1].bytes, b"MHD=1+ORDHDR:9");
        assert_eq!(shaped.parts[4].bytes, b"CDT=5000000000001:BOB?'S+SHOP");
        assert_eq!(shaped.parts[6].bytes, b"END=1");
        assert_eq!(
            shaped.parts[0].media_type.as_deref(),
            Some("application/edi-consent")
        );
        assert_eq!(shaped.message_type.as_deref(), Some("ORDHDR"));

        let typed = Tradacoms
            .shape(&stream(
                ORDERS,
                Some("application/edi-consent; charset=us-ascii"),
            ))
            .expect("well-formed");
        assert_eq!(
            typed.parts[0].media_type.as_deref(),
            Some("application/edi-consent; charset=us-ascii")
        );
    }

    #[test]
    fn the_tag_is_read_before_its_equals_sign_and_without_mhd_nothing_is_announced() {
        let shaped = Tradacoms
            .shape(&stream(b"STX=ANA:1+S+R+240101:120000+1'END=0'", None))
            .expect("well-formed");
        assert_eq!(shaped.parts.len(), 2);
        assert_eq!(shaped.parts[0].name.as_deref(), Some("STX"));
        assert_eq!(shaped.message_type, None);

        let segments = segment::segments(ORDERS, &DELIMITERS).expect("well-formed");
        assert_eq!(segments[0].element(0, &DELIMITERS), Some(&b"ANA:1"[..]));
        assert_eq!(
            segments[0].component(6, 0, &DELIMITERS),
            Some(&b"ORDHDR"[..])
        );
        assert_eq!(
            segments[4].component(0, 1, &DELIMITERS),
            Some(&b"BOB?'S"[..])
        );
        assert_eq!(segments[4].element(1, &DELIMITERS), Some(&b"SHOP"[..]));
        assert_eq!(DELIMITERS.unreleased(b"BOB?'S"), b"BOB'S");
    }

    #[test]
    fn a_cut_segment_or_an_untagged_one_or_an_empty_stream_is_refused_where_it_fails() {
        let cut = Tradacoms
            .shape(&stream(b"STX=ANA:1+S+R'MHD=1+ORDHDR:9", None))
            .expect_err("no terminator");
        assert_eq!(cut.offset, Some(14));
        assert_eq!(
            cut.to_string(),
            "edi-tradacoms: a segment without its terminator at byte 14"
        );

        let untagged = Tradacoms
            .shape(&stream(b"STX=ANA:1'=1'", None))
            .expect_err("no tag");
        assert_eq!(untagged.offset, Some(10));
        assert_eq!(untagged.reason, "a segment without a tag");

        let empty = Tradacoms.shape(&stream(b"", None)).expect_err("nothing");
        assert_eq!(empty.reason, "no segment at all");
        assert_eq!(empty.technology, "edi-tradacoms");
    }

    #[test]
    fn the_shape_claims_edi_consent_and_recognises_stx_at_the_head() {
        assert_eq!(Tradacoms.technology(), "edi-tradacoms");
        assert_eq!(Tradacoms.media_types(), &["application/edi-consent"]);
        assert!(Tradacoms.recognises(b"STX=ANA:1"));
        assert!(!Tradacoms.recognises(b"STX+ANA"));
        assert!(!Tradacoms.recognises(b"UNB+"));
        assert!(!Tradacoms.recognises(b""));

        let shapes: [&dyn Shape; 1] = [&Tradacoms];
        let by_media = message::choose(&shapes, &stream(b"x", Some("application/EDI-Consent")));
        assert_eq!(by_media.map(Shape::technology), Some("edi-tradacoms"));
        let by_look = message::choose(&shapes, &stream(ORDERS, None));
        assert_eq!(by_look.map(Shape::technology), Some("edi-tradacoms"));
        assert!(message::choose(&shapes, &stream(b"ISA*", None)).is_none());
    }
}
