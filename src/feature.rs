//! Interval-based annotation features.

use std::collections::HashMap;
use std::iter::FromIterator;

use bio::data_structures::interval_tree::{IntervalTree, IntervalTreeIterator};
use bio::io::Strand;
use bio::utils::{Interval, IntervalError};


pub trait NamedInterval: Sized {

    /// Underlying interval struct.
    fn interval(&self) -> &Interval<u64>;  // TODO: Generalize over interval types.

    /// Name of the interval.
    fn name(&self) -> Option<&str>;

    /// Name setter that returns the implementor itself.
    ///
    /// This function is expected to mutate the implementing type.
    fn with_name<T: Into<String>>(self, name: T) -> Self;

    /// Coordinate setter that returns the implementor itself.
    ///
    /// This function is expected to mutate the implementing type.
    fn with_coords(self, start: u64, end: u64) -> Result<Self, IntervalError>;

    /// Start coordinate of the interval.
    fn start(&self) -> u64 {
        self.interval().start
    }

    /// End coordinate of the interval.
    fn end(&self) -> u64 {
        self.interval().end
    }

    /// The number of bases covered by the interval.
    fn span(&self) -> u64 {
        self.end() - self.start()
    }

    /// Whether two intervals have an overlap or not.
    fn overlaps(&self, other: &Self) -> bool {
        self.start() < other.end() && other.start() < self.end()
    }

    /// Whether one interval completely contains the other.
    fn envelops(&self, other: &Self) -> bool {
        self.start() <= other.start() && self.end() >= other.end()
    }

    /// Whether two intervals cover a contiguous region without any overlaps.
    fn is_adjacent(&self, other: &Self) -> bool {
        self.end() == other.start() || self.start() == other.end()
    }
}

/// Macro for default function implementations of interval types.
macro_rules! impl_ninterval {
    ($struct_ty:ty) => (

        impl NamedInterval for $struct_ty {

            /// Name of the interval.
            fn name(&self) -> Option<&str> {
                self.name.as_ref().map(|n| n.as_str())
            }

            fn with_name<T>(mut self, name: T) -> $struct_ty
                where T: Into<String>
            {
                self.name = Some(name.into());
                self
            }

            fn interval(&self) -> &Interval<u64> {
                &self.interval
            }

            fn with_coords(mut self, start: u64, end: u64) -> Result<$struct_ty, IntervalError> {
                Interval::new(start..end)
                    .map(|iv| {
                        self.interval = iv;
                        self
                    })
            }
        }

    );
}

/// Default implementation of the `Interval` trait.
///
/// This struct also provides static methods for creating exons, transcripts, and genes.
#[derive(Debug)]
pub struct Feature {
    interval: Interval<u64>,
    name: Option<String>,
}

impl Default for Feature {

    fn default() -> Feature {
        Feature { interval: Interval::new(0..0).unwrap(), name: None }
    }
}

impl Feature {

    /// Creates a gene interval with default values.
    ///
    /// A gene interval is a container for transcript intervals.
    ///
    /// # Examples
    ///
    /// ```
    /// let gene = Feature::gene();
    ///
    /// assert_eq!(gene.transcript().len(), 0);
    /// assert_eq!(gene.start(), 0);
    /// assert_eq!(gene.end(), 0);
    /// assert!(gene.name().is_none());
    /// ```
    pub fn gene() -> Gene {
        Gene::default()
    }

    /// Creates a transcript interval with default values.
    ///
    /// A transcript interval is a container for exon intervals.
    ///
    /// # Examples
    ///
    /// ```
    /// use bio::io::Strand;
    ///
    /// let transcript = Feature::transcript();
    ///
    /// assert_eq!(transcript.exons().len(), 0);
    /// assert_eq!(transcript.strand(), &Strand::Unknown)
    /// assert_eq!(transcript.start(), 0);
    /// assert_eq!(transcript.end(), 0);
    /// assert!(transcript.name().is_none());
    /// ```
    pub fn transcript() -> Transcript {
        Transcript::default()
    }

    /// Creates an exon interval with default values.
    ///
    /// # Examples
    ///
    /// ```
    /// let exon = Feature::exon();
    ///
    /// assert_eq!(exon.start(), 0);
    /// assert_eq!(exon.end(), 0);
    /// assert!(exon.name().is_none());
    /// ```
    pub fn exon() -> Exon {
        Exon::default()
    }
}

impl_ninterval!(Feature);

/// Exon annotation.
#[derive(Debug)]
pub struct Exon {
    interval: Interval<u64>,
    name: Option<String>,
}

impl Default for Exon {

    fn default() -> Exon {
        Exon { interval: Interval::new(0..0).unwrap(), name: None }
    }
}

impl_ninterval!(Exon);

/// Transcript annotation.
#[derive(Debug)]
pub struct Transcript {
    interval: Interval<u64>,
    name: Option<String>,
    strand: Strand,
    cds: Option<Feature>,
    exons: IntervalTree<u64, Exon>,
}

impl Default for Transcript {

    fn default() -> Transcript {
        Transcript {
            interval: Interval::new(0..0).unwrap(),
            name: None,
            strand: Strand::Unknown,
            cds: None,
            exons: IntervalTree::new(),
        }
    }
}

impl_ninterval!(Transcript);

impl Transcript {

    pub fn strand(&self) -> &Strand {
        &self.strand
    }

    pub fn with_strand(mut self, strand: Strand) -> Transcript {
        self.strand = strand;
        self
    }

    pub fn exons(&self) -> &IntervalTree<u64, Exon> {
        &self.exons
    }

    pub fn with_exons<I>(mut self, exons: I) -> Transcript
        where I: IntoIterator<Item=Exon>
    {
        let exp_iter = exons.into_iter()
            .map(|exn| (exn.interval.clone(), exn));
        self.exons = IntervalTree::from_iter(exp_iter);
        self
    }

    pub fn insert_exon(&mut self, exon: Exon) {
        self.exons.insert(exon.start()..exon.end(), exon);
    }

    pub fn cds(&self) -> Option<&Feature> {
        self.cds.as_ref()
    }

    pub fn with_cds(mut self, cds: Feature) -> Transcript {
        self.cds = Some(cds);
        self
    }

    pub fn iter(&self) -> IntervalTreeIterator<u64, Exon> {
        // FIXME: iteration over IntervalTree items instead of using (min, max) hack.
        self.exons.find(0..u64::max_value())
    }

    /// Returns the number of exons in the trancripts in O(n) time.
    pub fn len(&self) -> usize {
        self.iter().map(|_exon| 1).fold(0, |acc, x| acc + x)
    }
}

/// Gene annotation.
#[derive(Debug)]
pub struct Gene {
    interval: Interval<u64>,
    name: Option<String>,
    transcripts: HashMap<String, Transcript>,
}

impl Default for Gene {

    fn default() -> Gene {
        Gene { interval: Interval::new(0..0).unwrap(),
               name: None,
               transcripts: HashMap::new(),
        }
    }
}

impl_ninterval!(Gene);

impl Gene {

    pub fn transcripts(&self) -> &HashMap<String, Transcript> {
        &self.transcripts
    }
}

#[cfg(test)]
mod test_feature {
    use super::*;

    #[test]
    fn default() {
        let fx = Feature::default();
        assert_eq!(fx.start(), 0);
        assert_eq!(fx.end(), 0);
        assert!(fx.name().is_none());
    }

    #[test]
    fn with_name() {
        let fx1 = Feature::default()
            .with_name("fx1");
        assert_eq!(fx1.start(), 0);
        assert_eq!(fx1.end(), 0);
        assert_eq!(fx1.name(), Some("fx1"));

        let fx2 = Feature::default()
            .with_name("fx2".to_owned());
        assert_eq!(fx2.start(), 0);
        assert_eq!(fx2.end(), 0);
        assert_eq!(fx2.name(), Some("fx2"));
    }

    #[test]
    fn with_coords() {
        let fxm = Feature::default()
            .with_coords(1, 3);
        assert!(fxm.is_ok());
        let fx = fxm.unwrap();
        assert_eq!(fx.start(), 1);
        assert_eq!(fx.end(), 3);
        assert!(fx.name().is_none());
    }

    #[test]
    fn with_coords_err() {
        let fxm = Feature::default()
            .with_coords(3, 1);
        assert!(fxm.is_err());
    }

    #[test]
    fn with_multiples() {
        let fxm = Feature::default()
            .with_coords(20, 30)
            .map(|f| f.with_name("fx"));
        assert!(fxm.is_ok());
        let fx = fxm.unwrap();
        assert_eq!(fx.start(), 20);
        assert_eq!(fx.end(), 30);
        assert_eq!(fx.name(), Some("fx"));
    }

    fn make_feature(start: u64, end: u64) -> Feature {
        Feature::default().with_coords(start, end).unwrap()
    }

    #[test]
    fn span() {
        let fx = make_feature(0, 15);
        assert_eq!(fx.span(), 15);
    }

    #[test]
    fn overlaps() {
        let fx1 = make_feature(100, 115);

        let fx2 = make_feature(110, 120);
        assert!(fx1.overlaps(&fx2));
        assert!(fx1.overlaps(&fx2));

        let fx3 = make_feature(115, 120);
        assert!(!fx1.overlaps(&fx3));
        assert!(!fx3.overlaps(&fx1));

        let fx4 = make_feature(90, 100);
        assert!(!fx1.overlaps(&fx4));
        assert!(!fx4.overlaps(&fx1));

        let fx5 = make_feature(200, 300);
        assert!(!fx1.overlaps(&fx5));
        assert!(!fx5.overlaps(&fx1));
    }

    #[test]
    fn envelops() {
        let fx1 = make_feature(100, 120);

        let fx2 = make_feature(105, 115);
        assert!(fx1.envelops(&fx2));
        assert!(!fx2.envelops(&fx1));

        let fx3 = make_feature(100, 105);
        assert!(fx1.envelops(&fx3));
        assert!(!fx3.envelops(&fx1));

        let fx4 = make_feature(115, 120);
        assert!(fx1.envelops(&fx4));
        assert!(!fx4.envelops(&fx1));

        let fx5 = make_feature(90, 105);
        assert!(!fx1.envelops(&fx5));
        assert!(!fx5.envelops(&fx1));

        let fx6 = make_feature(115, 130);
        assert!(!fx1.envelops(&fx5));
        assert!(!fx6.envelops(&fx1));

        let fx7 = make_feature(80, 90);
        assert!(!fx1.envelops(&fx7));
        assert!(!fx7.envelops(&fx1));
    }

    #[test]
    fn is_adjacent() {
        let fx1 = make_feature(100, 120);

        let fx2 = make_feature(90, 100);
        assert!(fx1.is_adjacent(&fx2));
        assert!(fx2.is_adjacent(&fx1));

        let fx3 = make_feature(120, 130);
        assert!(fx1.is_adjacent(&fx3));
        assert!(fx3.is_adjacent(&fx1));

        let fx4 = make_feature(90, 99);
        assert!(!fx1.is_adjacent(&fx4));
        assert!(!fx4.is_adjacent(&fx1));

        let fx5 = make_feature(119, 130);
        assert!(!fx1.is_adjacent(&fx5));
        assert!(!fx5.is_adjacent(&fx1));

        let fx6 = make_feature(100, 110);
        assert!(!fx1.is_adjacent(&fx6));
        assert!(!fx6.is_adjacent(&fx1));

        let fx7 = make_feature(110, 120);
        assert!(!fx1.is_adjacent(&fx7));
        assert!(!fx7.is_adjacent(&fx1));
    }
}

#[cfg(test)]
mod test_exon {
    use super::*;

    #[test]
    fn default() {
        let exon = Feature::exon();
        assert_eq!(exon.start(), 0);
        assert_eq!(exon.end(), 0);
        assert!(exon.name().is_none());
    }
}

#[cfg(test)]
mod test_transcript {
    use super::*;

    #[test]
    fn default() {
        let trx = Feature::transcript();
        assert_eq!(trx.start(), 0);
        assert_eq!(trx.end(), 0);
        assert!(trx.name().is_none());
        assert_eq!(trx.strand(), &Strand::Unknown);
        assert!(trx.cds().is_none());
        assert_eq!(trx.len(), 0);
    }

    #[test]
    fn with_strand() {
        let trx = Feature::transcript()
            .with_strand(Strand::Forward);
        assert_eq!(trx.strand(), &Strand::Forward);
    }

    fn make_exon<T: Into<String>>(start: u64, end: u64, name: T) -> Exon {
        Feature::exon()
            .with_name(name)
            .with_coords(start, end).unwrap()
    }

    #[test]
    fn with_exons() {
        let trx = Feature::transcript()
            .with_exons(vec![
                make_exon(1, 2, "ex1"),
                make_exon(10, 20, "ex2"),
                make_exon(100, 200, "ex3"),
            ]);
        assert_eq!(trx.len(), 3);
    }

    #[test]
    fn insert_exon() {
        let mut trx = Feature::transcript();
        assert_eq!(trx.len(), 0);
        trx.insert_exon(make_exon(1, 2, "ex"));
        assert_eq!(trx.len(), 1);
    }
}

#[cfg(test)]
mod test_gene {
    use super::*;

    #[test]
    fn default() {
        let gene = Feature::gene();
        assert_eq!(gene.start(), 0);
        assert_eq!(gene.end(), 0);
        assert!(gene.name().is_none());
        assert_eq!(gene.transcripts().len(), 0);
    }
}
