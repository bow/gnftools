//! Interval-based annotation features.

extern crate bio;

use std::cmp::{max, min};
use std::error::Error;
use std::collections::HashMap;
use std::fmt::{self, Display};

use bio::utils::{Interval, IntervalError, Strand, StrandError};

use self::error::FeatureError;


pub mod error {

    use super::*;

    #[derive(Debug, PartialEq)]
    pub enum FeatureError {
        IntervalError,
        StrandCharError,
        ConflictingStrandError,
        UnspecifiedStrandError,
        SubFeatureIntervalError,
        IncompleteTranscriptError,
    }

    impl Error for FeatureError {

        fn description(&self) -> &str {
            match *self {
                FeatureError::IntervalError =>
                    "interval start coordinate larger than its end coordinate",
                FeatureError::StrandCharError =>
                    "strand character is invalid",
                FeatureError::ConflictingStrandError =>
                    "conflicting strand inputs specified",
                FeatureError::UnspecifiedStrandError =>
                    "strand not specified",
                FeatureError::SubFeatureIntervalError =>
                    "subfeature interval is not completely enveloped by parent",
                FeatureError::IncompleteTranscriptError =>
                    "transcript annotation is incomplete",
            }
        }

        fn cause(&self) -> Option<&Error> {
            None
        }
    }

    impl Display for FeatureError {

        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "FeatureError: {}", self.description())
        }
    }

    impl From<IntervalError> for FeatureError {
        fn from(err: IntervalError) -> FeatureError {
            FeatureError::IntervalError
        }
    }

    impl From<StrandError> for FeatureError {
        fn from(_err: StrandError) -> FeatureError {
            FeatureError::StrandCharError
        }
    }
}

pub trait Annotation {
    fn seq_name(&self) -> &str;
    fn interval(&self) -> &Interval<u64>;
    fn attributes(&self) -> &HashMap<String, String>;
    fn attribute(&self, key: &str) -> Option<&str>;
    fn strand(&self) -> &Strand;
    fn span(&self) -> u64 {
        self.interval().end - self.interval().start
    }
}

macro_rules! impl_annotation {
    ($struct_ty:ty) => (

        impl Annotation for $struct_ty {

            fn seq_name(&self) -> &str {
                self.seq_name.as_str()
            }

            fn interval(&self) -> &Interval<u64> {
                &self.interval
            }

            fn attributes(&self) -> &HashMap<String, String> {
                &self.attributes
            }

            fn attribute(&self, key: &str) -> Option<&str> {
                self.attributes.get(key).map(|n| n.as_str())
            }

            fn strand(&self) -> &Strand {
                &self.strand
            }
        }

    );
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TxFeature {
    Exon,
    UTR,
    UTR5,
    UTR3,
    CDS,
    StartCodon,
    StopCodon,
    Any,
}

fn resolve_strand_input(strand: Option<Strand>, strand_char: Option<char>) -> Result<Strand, FeatureError> {
    match (strand, strand_char) {
        (None, None) => Err(FeatureError::UnspecifiedStrandError),
        (Some(sv), None) => Ok(sv),
        (None, Some(ref scv)) => Strand::from_char(scv).map_err(FeatureError::from),
        (Some(sv), Some(ref scv)) => {
            let sv_from_char = Strand::from_char(scv).map_err(FeatureError::from)?;
            if sv == sv_from_char {
                Ok(sv)
            } else {
                Err(FeatureError::ConflictingStrandError)
            }
        }
    }
}

fn coords_to_interval(start: u64, end: u64) -> Result<Interval<u64>, FeatureError> {
    Interval::new(start..end).map_err(FeatureError::from)
}

fn resolve_transcript_features(
    transcript_seqname: &String,
    transcript_interval: &Interval<u64>,
    transcript_strand: &Strand,
    features: Option<Vec<TranscriptFeature>>,
    exon_coords: Option<&Vec<(u64, u64)>>,
    cds_coord: Option<(u64, u64)>
) -> Result<Vec<TranscriptFeature>, FeatureError>
{
    // Deliberately not handling all possible input types to avoid
    // overcomplicating code. The inputs are expected to come from
    // either GTF or refFlat after all.

    match (features, exon_coords, cds_coord) {
        // nothing defined -> the transcript doesn't have any known subfeatures
        (None, None, None) => Ok(Vec::new()),

        // only CDS defined -> must be an error
        (None, None, Some(_)) => Err(FeatureError::IncompleteTranscriptError),

        // features defined ~ takes precedence over coords (GTF input, since we need
        // to construct the tx features first to store its annotations)
        // TODO: Maybe do some checks to ensure the given features are correct?
        (Some(fxs), _, _) => Ok(fxs.into_iter().collect()),

        // exon defined & coords possibly defined (refFlat input)
        (None, Some(raw_exon_coords), cdsc) =>
            infer_features(transcript_seqname, transcript_interval,
                           transcript_strand, raw_exon_coords, cdsc),
    }
}

pub fn infer_features(
    transcript_seqname: &String,
    transcript_interval: &Interval<u64>,
    transcript_strand: &Strand,
    exon_coords: &Vec<(u64, u64)>,
    cds_span: Option<(u64, u64)>
) -> Result<Vec<TranscriptFeature>, FeatureError>
{

    if exon_coords.len() == 0 {
        return Err(FeatureError::IncompleteTranscriptError);
    }

    let mut m_exon_coords = Vec::with_capacity(exon_coords.len());
    for &(a, b) in exon_coords.iter() {
        if a >= b {
            return Err(FeatureError::SubFeatureIntervalError)
        }
        m_exon_coords.push((a, b));
    }

    let exon_r = (m_exon_coords.first().unwrap().0, m_exon_coords.last().unwrap().1);

    if exon_r.0 != transcript_interval.start || exon_r.1 != transcript_interval.end {
        return Err(FeatureError::SubFeatureIntervalError);
    }

    match cds_span {

        // Improper CDS interval is an error
        Some(cds_r) if cds_r.0 > cds_r.1 =>
            Err(FeatureError::SubFeatureIntervalError),

        // Presence of proper CDS interval means we can resolve UTRs and start/stop codons
        Some(cds_r) if cds_r.0 < cds_r.1 => {
            // CDS coords must be fully enveloped by exon max-min
            if cds_r.0 < exon_r.0 || cds_r.1 > exon_r.1 {
                return Err(FeatureError::SubFeatureIntervalError);
            }
            // TODO: no exon start == CDS end and no exon end == CDS start
            // Rough heuristic: num of features (incl exons) ~ 2 * num of exons + 4
            let mut features: Vec<TranscriptFeature> =
                Vec::with_capacity(m_exon_coords.len() * 2 + 4);
            let (utr1, mcodon1) = match *transcript_strand {
                Strand::Forward => (TxFeature::UTR5, Some(TxFeature::StartCodon)),
                Strand::Reverse => (TxFeature::UTR3, Some(TxFeature::StopCodon)),
                Strand::Unknown => (TxFeature::UTR, None),
            };
            let (utr2, mcodon2) = match *transcript_strand {
                Strand::Forward => (TxFeature::UTR3, Some(TxFeature::StopCodon)),
                Strand::Reverse => (TxFeature::UTR5, Some(TxFeature::StartCodon)),
                Strand::Unknown => (TxFeature::UTR, None),
            };
            let make_feature = |kind, start, end| {
                TranscriptFeature {
                    seq_name: transcript_seqname.clone(),
                    kind: kind,
                    interval: Interval::new(start..end).unwrap(),
                    strand: *transcript_strand,
                    attributes: HashMap::new(),
                }
            };

            let (mut codon1_remaining, mut codon2_remaining) = (3, 3);
            for &(start, end) in m_exon_coords.iter() {

                // Whole UTR exon blocks
                if end <= cds_r.0 {
                    features.push(make_feature(TxFeature::Exon, start, end));
                    features.push(make_feature(utr1, start, end));

                // UTR-CDS exon block
                } else if start < cds_r.0 {
                    features.push(make_feature(TxFeature::Exon, start, end));
                    features.push(make_feature(utr1, start, cds_r.0));
                    if let Some(codon1) = mcodon1 {
                        let codon = make_feature(codon1, cds_r.0, min(cds_r.0 + 3, end));
                        codon1_remaining -= codon.span();
                        features.push(codon);
                    }
                    features.push(make_feature(TxFeature::CDS, cds_r.0, end));

                // Whole CDS exon blocks
                } else if end < cds_r.1 {
                    features.push(make_feature(TxFeature::Exon, start, end));
                    if codon1_remaining > 0 {
                        if let Some(codon1) = mcodon1 {
                            let codon =
                                make_feature(codon1, start, min(start + codon1_remaining, end));
                            codon1_remaining -= codon.span();
                            features.push(codon);
                        }
                    }
                    features.push(make_feature(TxFeature::CDS, start, end));

                // Whole CDS exon blocks at the end
                } else if end == cds_r.1 {
                    features.push(make_feature(TxFeature::Exon, start, end));
                    features.push(make_feature(TxFeature::CDS, start, end));
                    if end - start >= codon2_remaining {
                        if let Some(codon2) = mcodon2 {
                            let codon =
                                make_feature(codon2, max(start, cds_r.1 - codon2_remaining), end);
                            codon2_remaining -= codon.span();
                            features.push(codon);
                        }
                    }

                // CDS-UTR exon block
                } else if start < cds_r.1 {
                    // FIXME: This will break when a stop codon is split into 3.
                    //        Not to mention it looks ugly :(.
                    if cds_r.1 - start < codon2_remaining {
                        let prev_end = match features.last().as_ref() {
                            Some(&fx) => fx.interval().end,
                            _ => 0,
                        };
                        if prev_end > 0 {
                            if let Some(codon2) = mcodon2 {
                                let codon = make_feature(
                                    codon2, prev_end - (codon2_remaining - (cds_r.1 - start)),
                                    prev_end);
                                codon2_remaining -= codon.span();
                                features.push(codon);
                            }
                        }
                    }
                    features.push(make_feature(TxFeature::Exon, start, end));
                    features.push(make_feature(TxFeature::CDS, start, cds_r.1));
                    if let Some(codon2) = mcodon2 {
                        let codon =
                            make_feature(codon2, max(start, cds_r.1 - codon2_remaining), cds_r.1);
                        codon2_remaining -= codon.span();
                        features.push(codon);
                    }
                    features.push(make_feature(utr2, cds_r.1, end));

                // Whole UTR exon blocks
                } else {
                    features.push(make_feature(TxFeature::Exon, start, end));
                    features.push(make_feature(utr2, start, end));
                }
            }

            Ok(features)
        },

        // No CDS intervals mean we just sort the coordinates and create the exons
        _ => {
            let mut features = Vec::with_capacity(m_exon_coords.len());
            for &(start, end) in m_exon_coords.iter() {
                if start > end {
                    return Err(FeatureError::SubFeatureIntervalError)
                }
                features.push(
                    TranscriptFeature {
                        seq_name: transcript_seqname.clone(),
                        kind: TxFeature::Exon,
                        interval: Interval::new(start..end).unwrap(),
                        strand: *transcript_strand,
                        attributes: HashMap::new(),
                    });
            }
            Ok(features)
        }
    }
}

#[cfg(test)]
mod test_transcript {
    use super::*;
    use self::TxFeature::*;

    fn get_coords_by_feature(fxs: &Vec<TranscriptFeature>, kind: TxFeature) -> Vec<(u64, u64)> {
        fxs.iter()
            .filter(|fx| *fx.kind() == kind)
            .map(|fx| (fx.interval().start, fx.interval().end))
            .collect()
    }

    fn get_features(fxs: &Vec<TranscriptFeature>) -> Vec<TxFeature> {
        fxs.iter().map(|fx| *fx.kind()).collect()
    }

    #[test]
    fn builder_from_coords() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], None)
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(fxs.iter().filter(|fx| *fx.kind() == Exon).count(), 3);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((200, 900)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, CDS, Exon, CDS, StopCodon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 200)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(200, 203)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(200, 300), (400, 500), (700, 900)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(897, 900)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(900, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_start_codon_at_end() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((297, 900)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, CDS, Exon, CDS, StopCodon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 297)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(297, 300)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(297, 300), (400, 500), (700, 900)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(897, 900)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(900, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_start_codon_split() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((299, 900)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, StartCodon, CDS, Exon, CDS,
                        StopCodon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 299)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(299, 300), (400, 402)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(299, 300), (400, 500), (700, 900)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(897, 900)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(900, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_start_codon_at_start() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((400, 900)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, Exon, StartCodon, CDS, Exon, CDS, StopCodon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 300)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(400, 403)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(400, 500), (700, 900)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(897, 900)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(900, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_stop_codon_at_end() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((200, 500)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, CDS, StopCodon, Exon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 200)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(200, 203)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(200, 300), (400, 500)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(497, 500)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(700, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_stop_codon_split() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((200, 701)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, CDS, StopCodon, Exon, CDS, StopCodon,
                        UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 200)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(200, 203)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(200, 300), (400, 500), (700, 701)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(498, 500), (700, 701)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(701, 1000)]);
    }

    #[test]
    fn builder_from_coords_with_cds_fwd_stop_codon_at_start() {
        let tm = TranscriptBuilder::new("chrT", 100, 1000)
            .strand(Strand::Forward)
            .exon_and_cds_coords(vec![(100, 300), (400, 500), (700, 1000)], Some((200, 703)))
            .build();
        assert!(tm.is_ok(), "{:?}", tm);
        let t = tm.unwrap();
        let fxs = t.features();
        assert_eq!(get_features(fxs),
                   vec![Exon, UTR5, StartCodon, CDS, Exon, CDS, Exon, CDS, StopCodon, UTR3]);
        assert_eq!(get_coords_by_feature(fxs, Exon),
                   vec![(100, 300), (400, 500), (700, 1000)]);
        assert_eq!(get_coords_by_feature(fxs, UTR5), vec![(100, 200)]);
        assert_eq!(get_coords_by_feature(fxs, StartCodon), vec![(200, 203)]);
        assert_eq!(get_coords_by_feature(fxs, CDS), vec![(200, 300), (400, 500), (700, 703)]);
        assert_eq!(get_coords_by_feature(fxs, StopCodon), vec![(700, 703)]);
        assert_eq!(get_coords_by_feature(fxs, UTR3), vec![(703, 1000)]);
    }
}

#[derive(Debug)]
pub struct TranscriptFeature {
    kind: TxFeature,
    seq_name: String,
    interval: Interval<u64>,
    strand: Strand,
    attributes: HashMap<String, String>
}

impl_annotation!(TranscriptFeature);

impl TranscriptFeature {

    pub fn kind(&self) -> &TxFeature {
        &self.kind
    }
}

pub struct TxFeatureBuilder {
    seq_name: String,
    start: u64,
    end: u64,
    attributes: HashMap<String, String>,
    kind: TxFeature,
    strand: Option<Strand>,
    strand_char: Option<char>,
}

impl TxFeatureBuilder {

    pub fn new<T>(seq_name: T, start: u64, end: u64) -> TxFeatureBuilder
        where T: Into<String>
    {
        TxFeatureBuilder {
            seq_name: seq_name.into(),
            start: start, end: end,
            kind: TxFeature::Any,
            attributes: HashMap::new(),
            strand: None,
            strand_char: None,
        }
    }

    pub fn kind(mut self, kind: TxFeature) -> TxFeatureBuilder {
        self.kind = kind;
        self
    }

    pub fn strand(mut self, strand: Strand) -> TxFeatureBuilder {
        self.strand = Some(strand);
        self
    }

    pub fn strand_char(mut self, strand_char: char) -> TxFeatureBuilder {
        self.strand_char = Some(strand_char);
        self
    }

    pub fn attribute<K, V>(mut self, key: K, value: V) -> TxFeatureBuilder
        where K: Into<String>, V: Into<String>
    {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn build(self) -> Result<TranscriptFeature, FeatureError> {
        let interval = coords_to_interval(self.start, self.end)?;
        let strand = resolve_strand_input(self.strand, self.strand_char)?;
        let feature = TranscriptFeature {
            seq_name: self.seq_name, kind: self.kind, interval: interval,
            strand: strand, attributes: self.attributes,
        };
        Ok(feature)
    }
}

#[derive(Debug)]
pub struct Transcript {
    seq_name: String,
    interval: Interval<u64>,
    strand: Strand,
    attributes: HashMap<String, String>,
    features: Vec<TranscriptFeature>,
}

impl_annotation!(Transcript);

impl Transcript {

    pub fn features(&self) -> &Vec<TranscriptFeature> {
        &self.features
    }
}

pub struct TranscriptBuilder {
    seq_name: String,
    start: u64, end: u64,
    strand: Option<Strand>,
    strand_char: Option<char>,
    // Input can be a vector of pre-made features ...
    features: Option<Vec<TranscriptFeature>>,
    // Or exon coordinates, possibly coupled with cds coord
    // NOTE: Can we instead of using Vec<_> here keep it as an unconsumed iterator?
    exon_coords: Option<Vec<(u64, u64)>>,
    cds_coord: Option<(u64, u64)>,
    attributes: HashMap<String, String>,
}

impl TranscriptBuilder {

    pub fn new<T>(seq_name: T, start: u64, end: u64) -> TranscriptBuilder
        where T: Into<String>
    {
        TranscriptBuilder {
            seq_name: seq_name.into(),
            start: start, end: end,
            strand: None,
            strand_char: None,
            features: None,
            exon_coords: None,
            cds_coord: None,
            attributes: HashMap::new(),
        }
    }

    pub fn strand(mut self, strand: Strand) -> TranscriptBuilder {
        self.strand = Some(strand);
        self
    }

    pub fn strand_char(mut self, strand_char: char) -> TranscriptBuilder {
        self.strand_char = Some(strand_char);
        self
    }

    pub fn attribute<K, V>(mut self, key: K, value: V) -> TranscriptBuilder
        where K: Into<String>, V: Into<String>
    {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn exon_and_cds_coords<E>(
        mut self,
        exon_coords: E,
        cds_coord: Option<(u64, u64)>
    )-> TranscriptBuilder
        where E: IntoIterator<Item=(u64, u64)>
    {
        self.exon_coords = Some(exon_coords.into_iter().collect());
        self.cds_coord = cds_coord;
        self
    }

    pub fn build(mut self) -> Result<Transcript, FeatureError> {
        let interval = coords_to_interval(self.start, self.end)?;
        let strand = resolve_strand_input(self.strand, self.strand_char)?;
        let features = resolve_transcript_features(
            &self.seq_name, &interval, &strand,
            self.features, self.exon_coords.as_ref(), self.cds_coord)?;

        let transcript = Transcript {
            seq_name: self.seq_name, interval: interval, strand: strand,
            features: features, attributes: self.attributes,
        };
        Ok(transcript)
    }
}

#[derive(Debug)]
pub struct Gene {
    seq_name: String,
    interval: Interval<u64>,
    strand: Strand,
    attributes: HashMap<String, String>,
    transcripts: HashMap<String, Transcript>,
}

impl_annotation!(Gene);


#[cfg(test)]
mod test_transcript_feature {
    use super::*;

    #[test]
    fn builder() {
        let tfm1 = TxFeatureBuilder::new("chrT", 10, 20)
            .kind(TxFeature::Exon)
            .strand(Strand::Forward)
            .attribute("name", "ex1")
            .build();
        assert!(tfm1.is_ok());
        let tf = tfm1.unwrap();
        assert_eq!(tf.seq_name(), "chrT");
        assert_eq!(tf.kind(), &TxFeature::Exon);
        assert_eq!(tf.strand(), &Strand::Forward);
        assert_eq!(tf.attribute("name"), Some("ex1"));
        assert_eq!(tf.attributes.len(), 1);

        let tfm2 = TxFeatureBuilder::new("chrO", 10, 10)
            .strand_char('-')
            .strand(Strand::Reverse)
            .build();
        assert!(tfm2.is_ok());
    }

    #[test]
    fn builder_interval_invalid() {
        let tfm = TxFeatureBuilder::new("chrE", 20, 10).build();
        assert!(tfm.is_err());
        assert_eq!(tfm.unwrap_err(), FeatureError::IntervalError);
    }

    #[test]
    fn builder_strand_unspecified() {
        let tfm = TxFeatureBuilder::new("chrT", 20, 30)
            .build();
        assert!(tfm.is_err());
        assert_eq!(tfm.unwrap_err(), FeatureError::UnspecifiedStrandError);
    }

    #[test]
    fn builder_strand_char_unexpected() {
        let tfm = TxFeatureBuilder::new("chrE", 10, 20)
            .strand_char('w')
            .build();
        assert!(tfm.is_err());
        assert_eq!(tfm.unwrap_err(), FeatureError::StrandCharError);
    }

    #[test]
    fn builder_strand_char_conflicting() {
        let tfm = TxFeatureBuilder::new("chrE", 10, 20)
            .strand_char('-')
            .strand(Strand::Reverse)
            .build();
        assert!(tfm.is_ok());
        let tf = tfm.unwrap();
        assert_eq!(tf.strand(), &Strand::Reverse);
    }
}