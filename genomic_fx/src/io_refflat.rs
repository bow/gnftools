//! Reader and writer for the refFlat format.
//!
//! The refFlat format is a transcript-oriented format in which each
//! transcript is denoted in a single line. It is used most prominently
//! by the [picard suite tools](https://broadinstitute.github.io/picard/)
//!
//! A minimum specification of the columns can be found here:
//! https://genome.ucsc.edu/goldenPath/gbdDescriptionsOld.html#RefFlat


use std::cmp::{max, min};
use std::convert::AsRef;
use std::io;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use csv;
use itertools::{GroupBy, Itertools};
use linked_hash_map::LinkedHashMap;

use feature::FeatureError;
use {Coord, Gene, GBuilder, Strand, Transcript, TBuilder, Error};


pub type RefFlatRow = (String, String, String, char, u64, u64, u64, u64, usize, String, String);

pub struct RefFlatRecord {
    pub gene_id: String,
    pub transcript_name: String,
    pub seq_name: String,
    pub strand: char,
    pub trx_start: u64,
    pub trx_end: u64,
    pub coding_start: u64,
    pub coding_end: u64,
    pub num_exons: usize,
    pub exon_starts: String,
    pub exon_ends: String,
}

impl RefFlatRecord {

    pub fn into_transcript(self) -> Result<Transcript, Error> {

        let exon_coords = self.zip_raw_exon_coords()?;
        if exon_coords.len() != self.num_exons {
            return Err(Error::RefFlat(
                "number of exon and exon coordinates mismatch"));
        }

        let strand = Strand::from_char(&self.strand)
            .map_err(FeatureError::from)
            .map_err(Error::from)?;

        let coding_coord =
            if self.coding_start != self.coding_end {
                Some((self.coding_start, self.coding_end))
            } else {
                None
            };

        TBuilder::new(self.seq_name, self.trx_start, self.trx_end)
            .id(self.transcript_name)
            .strand(strand)
            .coords(exon_coords, coding_coord)
            .coding_incl_stop(true)
            .attribute("gene_id", self.gene_id)
            .build()
            .map_err(Error::from)
    }

    #[inline]
    fn zip_raw_exon_coords(&self) -> Result<Vec<Coord<u64>>, Error> {
        let exon_starts = self.exon_starts
            .trim_matches(',').split(',').map(|item| u64::from_str(item).map_err(Error::from));
        let exon_ends = self.exon_ends
            .trim_matches(',').split(',').map(|item| u64::from_str(item).map_err(Error::from));

        let mut res = vec![];
        for (rstart, rend) in exon_starts.zip(exon_ends) {
            let start = rstart?;
            let end = rend?;
            res.push((start, end));
        }
        Ok(res)
    }
}

impl From<RefFlatRow> for RefFlatRecord {

    fn from(row: RefFlatRow) -> Self {
        RefFlatRecord {
            gene_id: row.0,
            transcript_name: row.1,
            seq_name: row.2,
            strand: row.3,
            trx_start: row.4,
            trx_end: row.5,
            coding_start: row.6,
            coding_end: row.7,
            num_exons: row.8,
            exon_starts: row.9,
            exon_ends: row.10,
        }
    }
}

pub struct Reader<R: io::Read> {
    inner: csv::Reader<R>,
}

impl<R: io::Read> Reader<R> {

    pub fn from_reader(in_reader: R) -> Reader<R> {
        Reader {
            inner: csv::Reader::from_reader(in_reader)
                .delimiter(b'\t')
                .has_headers(false)
        }
    }

    pub fn records<'a>(&'a mut self) -> RefFlatRecords<'a, R> {
        RefFlatRecords {
            inner: self.inner.decode()
        }
    }

    pub fn transcripts<'a>(&'a mut self) -> RefFlatTranscripts<'a, R> {
        RefFlatTranscripts {
            inner: self.records()
        }
    }

    pub fn genes<'a>(&'a mut self) -> RefFlatGenes<'a, R> {
        RefFlatGenes {
            inner: self.records().group_by(groupf),
        }
    }
}

impl Reader<fs::File> {
    pub fn from_file<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        fs::File::open(path).map(Reader::from_reader)
    }
}

pub struct RefFlatRecords<'a, R: 'a> where R: io::Read {
    inner: csv::DecodedRecords<'a, R, RefFlatRow>,
}

impl<'a, R> Iterator for RefFlatRecords<'a, R> where R: io::Read {

    type Item = Result<RefFlatRecord, Error>;

    fn next(&mut self) -> Option<Result<RefFlatRecord, Error>> {
        self.inner.next()
            .map(|row| {
                row.or_else(|err| Err(Error::from(err)))
                    .map(RefFlatRecord::from)
            })
    }
}

pub struct RefFlatTranscripts<'a, R: 'a> where R: io::Read {
    inner: RefFlatRecords<'a, R>,
}

impl<'a, R> Iterator for RefFlatTranscripts<'a, R> where R: io::Read {

    type Item = Result<Transcript, Error>;

    fn next(&mut self) -> Option<Result<Transcript, Error>> {
        self.inner.next()
            .map(|record| record.and_then(|rec| rec.into_transcript()))
    }
}

pub struct RefFlatGenes<'a, R: 'a> where R: io::Read, {
    inner: GroupBy<GroupKey, RefFlatRecords<'a, R>, GroupFunc>,
}

impl<'a, R> Iterator for RefFlatGenes<'a, R> where R: io::Read {

    type Item = Result<Gene, Error>;

    fn next(&mut self) -> Option<Result<Gene, Error>> {
        self.inner.into_iter()
            .map(|(okey, records)| {
                match okey {
                    Some((gid, seq_name, strand_char)) => {
                        let mut transcripts = LinkedHashMap::new();
                        let (mut gene_start, mut gene_end) = (u64::max_value(), u64::min_value());
                        for record in records {
                            let transcript = record.and_then(|rec| rec.into_transcript())?;
                            gene_start = min(gene_start, transcript.interval().start);
                            gene_end = max(gene_end, transcript.interval().end);
                            let tid = transcript.id.clone()
                                .ok_or(Error::RefFlat("transcript does not have ID"))?;
                            transcripts.insert(tid, transcript);
                        }
                        GBuilder::new(seq_name, gene_start, gene_end)
                            .id(gid)
                            .strand_char(strand_char)
                            .transcripts(transcripts)
                            .transcript_coding_incl_stop(true)
                            .build()
                    },
                    None => Err(records.filter_map(|x| x.err()).next().unwrap())
                }
            }).next()
    }
}

pub struct Writer<W: io::Write> {
    inner: csv::Writer<W>,
}

impl<W: io::Write> Writer<W> {

    pub fn from_writer(in_writer: W) -> Writer<W> {
        Writer {
            inner: csv::Writer::from_writer(in_writer)
                .delimiter(b'\t')
                .quote_style(csv::QuoteStyle::Never)
        }
    }

    pub fn write(&mut self, row: &RefFlatRow) -> Result<(), Error> {
        self.inner
            .encode((&row.0, &row.1, &row.2, row.3, row.4, row.5, row.6, row.7, row.8,
                     &row.9, &row.10))
            .map_err(Error::from)
    }

    pub fn write_record(&mut self, record: &RefFlatRecord) -> Result<(), Error> {
        self.inner
            .encode((&record.gene_id, &record.transcript_name, &record.seq_name,
                     record.strand, record.trx_start, record.trx_end,
                     record.coding_start, record.coding_end, record.num_exons,
                     &record.exon_starts, &record.exon_ends))
            .map_err(Error::from)
    }

    pub fn write_transcript(&mut self, transcript: &Transcript) -> Result<(), Error> {
        self.write_transcript_gid(&transcript, None)
    }

    pub fn write_gene(&mut self, gene: &Gene) -> Result<(), Error> {
        let gid = gene.id.as_ref().map(|v| v.as_ref());
        for (_, transcript) in gene.transcripts() {
            self.write_transcript_gid(&transcript, gid)?;
        }
        Ok(())
    }

    fn write_transcript_gid(&mut self, transcript: &Transcript, gid: Option<&str>)
    -> Result<(), Error>
    {
        let gene_id = match gid {
            Some(ref gid) => gid,
            None => transcript.attributes.get("gene_id").map(|gid| gid.as_ref()).unwrap_or(""),
        };
        let transcript_name = transcript.id.as_ref()
            .map(|tn| tn.as_ref()).unwrap_or("");
        let strand_char = match transcript.strand() {
            &Strand::Forward => '+',
            &Strand::Reverse => '-',
            &Strand::Unknown => '.',
        };

        let (coding_start, coding_end) = transcript.coding_coord(true)
            .unwrap_or((transcript.interval().end, transcript.interval().end));
        let (exon_starts, exon_ends) = Self::coords_field(&transcript);

        self.inner
            .encode((gene_id, transcript_name, transcript.seq_name(), strand_char,
                     transcript.interval().start, transcript.interval().end,
                     coding_start, coding_end, transcript.exons().len(),
                     exon_starts, exon_ends))
            .map_err(Error::from)
    }

    #[inline(always)]
    fn coords_field(trx: &Transcript) -> (String, String) {
        let mut starts = trx.exons().iter().map(|exon| exon.interval().start).join(",");
        starts.push(',');
        let mut ends = trx.exons().iter().map(|exon| exon.interval().end).join(",");
        ends.push(',');
        (starts, ends)
    }
}

impl Writer<fs::File> {

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let f = fs::File::create(path).map_err(Error::from)?;
        Ok(Writer::from_writer(f))
    }
}

type GroupKey = Option<(String, String, char)>;

type GroupFunc = fn(&Result<RefFlatRecord, Error>) -> GroupKey;

fn groupf(result: &Result<RefFlatRecord, Error>) -> GroupKey {
    result.as_ref().ok()
        .map(|ref res| (res.gene_id.clone(), res.seq_name.clone(), res.strand.clone()))
}


#[cfg(test)]
mod test_reader {

    use super::*;

    fn next_rec<'a, R>(rr: &mut RefFlatRecords<'a, R>) -> RefFlatRecord where R: io::Read {
        rr.next().expect("a refflat record result").expect("a refflat record")
    }

    fn next_trx<'a, R>(rt: &mut RefFlatTranscripts<'a, R>) -> Transcript where R: io::Read {
        rt.next().expect("a transcript result").expect("a transcript")
    }

    fn next_gx<'a, R>(rg: &mut RefFlatGenes<'a, R>) -> Gene where R: io::Read {
        rg.next().expect("a gene result").expect("a gene")
    }

    const REFFLAT_SINGLE_ROW_NO_CDS: &'static [u8] =  b"DDX11L1\tNR_046018\tchr1\t+\t11873\t14409\t14409\t14409\t3\t11873,12612,13220,\t12227,12721,14409,";

    #[test]
    fn records_single_row_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_SINGLE_ROW_NO_CDS);
        let mut records = reader.records();

        let rec1 = next_rec(&mut records);
        assert_eq!(rec1.gene_id, "DDX11L1".to_owned());

        assert!(records.next().is_none());
    }

    #[test]
    fn transcripts_single_row_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_SINGLE_ROW_NO_CDS);
        let mut transcripts = reader.transcripts();

        let trx1 = next_trx(&mut transcripts);
        assert_eq!(trx1.id, Some("NR_046018".to_owned()));

        assert!(transcripts.next().is_none());
    }

    #[test]
    fn genes_single_row_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_SINGLE_ROW_NO_CDS);
        let mut genes = reader.genes();

        let gx1 = next_gx(&mut genes);
        assert_eq!(gx1.id, Some("DDX11L1".to_owned()));

        assert!(genes.next().is_none());
    }

    const REFFLAT_MULT_ROWS_NO_CDS: &'static [u8] =  b"DDX11L1\tNR_046018\tchr1\t+\t11873\t14409\t14409\t14409\t3\t11873,12612,13220,\t12227,12721,14409,
MIR570\tNR_030296\tchr3\t+\t195699400\t195699497\t195699497\t195699497\t1\t195699400,\t195699497,";

    #[test]
    fn records_mult_rows_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_NO_CDS);
        let mut records = reader.records();

        let rec1 = next_rec(&mut records);
        assert_eq!(rec1.gene_id, "DDX11L1".to_owned());

        let rec2 = next_rec(&mut records);
        assert_eq!(rec2.gene_id, "MIR570".to_owned());

        assert!(records.next().is_none());
    }

    #[test]
    fn transcripts_mult_rows_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_NO_CDS);
        let mut transcripts = reader.transcripts();

        let trx1 = next_trx(&mut transcripts);
        assert_eq!(trx1.id, Some("NR_046018".to_owned()));

        let trx2 = next_trx(&mut transcripts);
        assert_eq!(trx2.id, Some("NR_030296".to_owned()));

        assert!(transcripts.next().is_none());
    }

    #[test]
    fn genes_mult_rows_no_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_NO_CDS);
        let mut genes = reader.genes();

        let gx1 = next_gx(&mut genes);
        assert_eq!(gx1.id, Some("DDX11L1".to_owned()));

        let gx2 = next_gx(&mut genes);
        assert_eq!(gx2.id, Some("MIR570".to_owned()));

        assert!(genes.next().is_none());
    }

    const REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS: &'static [u8] = b"TNFRSF14\tNM_001297605\tchr1\t+\t2556364\t2565622\t2556664\t2562868\t7\t2556364,2557725,2558342,2559822,2560623,2562864,2563147,\t2556733,2557834,2558468,2559978,2560714,2562896,2565622,
TNFRSF14\tNM_003820\tchr1\t+\t2556364\t2565622\t2556664\t2563273\t8\t2556364,2557725,2558342,2559822,2560623,2561672,2562864,2563147,\t2556733,2557834,2558468,2559978,2560714,2561815,2562896,2565622,
SMIM12\tNM_001164824\tchr1\t-\t34850361\t34859045\t34855698\t34855977\t3\t34850361,34856555,34858839,\t34855982,34856739,34859045,
SMIM12\tNM_001164825\tchr1\t-\t34850361\t34859737\t34855698\t34855977\t2\t34850361,34859454,\t34855982,34859737,
SMIM12\tNM_138428\tchr1\t-\t34850361\t34859816\t34855698\t34855977\t2\t34850361,34859676,\t34855982,34859816,";

    #[test]
    fn records_mult_rows_mult_genes_with_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS);
        let mut records = reader.records();

        let rec1 = next_rec(&mut records);
        assert_eq!(rec1.transcript_name, "NM_001297605".to_owned());

        let _rec2 = next_rec(&mut records);
        let _rec3 = next_rec(&mut records);
        let _rec4 = next_rec(&mut records);

        let rec5 = next_rec(&mut records);
        assert_eq!(rec5.transcript_name, "NM_138428".to_owned());

        assert!(records.next().is_none());
    }

    #[test]
    fn transcripts_mult_rows_mult_genes_with_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS);
        let mut transcripts = reader.transcripts();

        let trx1 = next_trx(&mut transcripts);
        assert_eq!(trx1.id, Some("NM_001297605".to_owned()));

        let _trx2 = next_trx(&mut transcripts);
        let _trx3 = next_trx(&mut transcripts);
        let _trx4 = next_trx(&mut transcripts);

        let trx5 = next_trx(&mut transcripts);
        assert_eq!(trx5.id, Some("NM_138428".to_owned()));

        assert!(transcripts.next().is_none());
    }

    #[test]
    fn genes_mult_rows_mult_genes_with_cds() {
        let mut reader = Reader::from_reader(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS);
        let mut genes = reader.genes();

        let gx1 = next_gx(&mut genes);
        assert_eq!(gx1.id, Some("TNFRSF14".to_owned()));

        let gx2 = next_gx(&mut genes);
        assert_eq!(gx2.id, Some("SMIM12".to_owned()));

        assert!(genes.next().is_none());
    }
}

#[cfg(test)]
mod test_writer {

    use super::*;

    const REFFLAT_SINGLE_ROW_NO_CDS: &'static [u8] =  b"DDX11L1\tNR_046018\tchr1\t+\t11873\t14409\t14409\t14409\t3\t11873,12612,13220,\t12227,12721,14409,\n";

    #[test]
    fn rows_single_row_no_cds() {
        let row =
            ("DDX11L1".to_owned(), "NR_046018".to_owned(), "chr1".to_owned(),
             '+', 11873, 14409, 14409, 14409, 3,
             "11873,12612,13220,".to_owned(), "12227,12721,14409,".to_owned());

        let mut writer = Writer::from_writer(vec![]);
        writer.write(&row).expect("a successful write");
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_SINGLE_ROW_NO_CDS));
    }

    #[test]
    fn records_single_row_no_cds() {
        let rec = RefFlatRecord {
            gene_id: "DDX11L1".to_owned(),
            transcript_name: "NR_046018".to_owned(),
            seq_name: "chr1".to_owned(),
            strand: '+',
            trx_start: 11873,
            trx_end: 14409,
            coding_start: 14409,
            coding_end: 14409,
            num_exons: 3,
            exon_starts: "11873,12612,13220,".to_owned(),
            exon_ends: "12227,12721,14409,".to_owned()
        };

        let mut writer = Writer::from_writer(vec![]);
        writer.write_record(&rec).expect("a successful write");
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_SINGLE_ROW_NO_CDS));
    }

    #[test]
    fn transcripts_single_row_no_cds() {
        let trx = TBuilder::new("chr1", 11873,  14409)
            .strand_char('+')
            .id("NR_046018")
            .attribute("gene_id", "DDX11L1")
            .coords(vec![(11873, 12227), (12612, 12721), (13220, 14409)], None)
            .coding_incl_stop(true)
            .build()
            .expect("a transcript");

        let mut writer = Writer::from_writer(vec![]);
        writer.write_transcript(&trx).expect("a successful write");
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_SINGLE_ROW_NO_CDS));
    }

    #[test]
    fn genes_single_row_no_cds() {
        let mut cs = Vec::new();
        cs.push(("NR_046018".to_owned(),
                 ((11873, 14409), vec![(11873, 12227), (12612, 12721), (13220, 14409)], None)));

        let gx = GBuilder::new("chr1", 11873, 14409)
            .strand_char('+')
            .id("DDX11L1")
            .transcript_coords(cs)
            .transcript_coding_incl_stop(true)
            .build()
            .expect("a gene");

        let mut writer = Writer::from_writer(vec![]);
        let res = writer.write_gene(&gx);
        assert!(res.is_ok(), "{:?}");
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_SINGLE_ROW_NO_CDS));
    }

    const REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS: &'static [u8] = b"TNFRSF14\tNM_001297605\tchr1\t+\t2556364\t2565622\t2556664\t2562868\t7\t2556364,2557725,2558342,2559822,2560623,2562864,2563147,\t2556733,2557834,2558468,2559978,2560714,2562896,2565622,
TNFRSF14\tNM_003820\tchr1\t+\t2556364\t2565622\t2556664\t2563273\t8\t2556364,2557725,2558342,2559822,2560623,2561672,2562864,2563147,\t2556733,2557834,2558468,2559978,2560714,2561815,2562896,2565622,
SMIM12\tNM_001164824\tchr1\t-\t34850361\t34859045\t34855698\t34855977\t3\t34850361,34856555,34858839,\t34855982,34856739,34859045,
SMIM12\tNM_001164825\tchr1\t-\t34850361\t34859737\t34855698\t34855977\t2\t34850361,34859454,\t34855982,34859737,
SMIM12\tNM_138428\tchr1\t-\t34850361\t34859816\t34855698\t34855977\t2\t34850361,34859676,\t34855982,34859816,\n";

    #[test]
    fn records_mult_rows_mult_genes_with_cds() {
        let recs = vec![
            RefFlatRecord {
                gene_id: "TNFRSF14".to_owned(),
                transcript_name: "NM_001297605".to_owned(),
                seq_name: "chr1".to_owned(),
                strand: '+',
                trx_start: 2556364,
                trx_end: 2565622,
                coding_start: 2556664,
                coding_end: 2562868,
                num_exons: 7,
                exon_starts: "2556364,2557725,2558342,2559822,2560623,2562864,2563147,".to_owned(),
                exon_ends: "2556733,2557834,2558468,2559978,2560714,2562896,2565622,".to_owned(),
            },
            RefFlatRecord {
                gene_id: "TNFRSF14".to_owned(),
                transcript_name: "NM_003820".to_owned(),
                seq_name: "chr1".to_owned(),
                strand: '+',
                trx_start: 2556364,
                trx_end: 2565622,
                coding_start: 2556664,
                coding_end: 2563273,
                num_exons: 8,
                exon_starts:
                    "2556364,2557725,2558342,2559822,2560623,2561672,2562864,2563147,".to_owned(),
                exon_ends:
                    "2556733,2557834,2558468,2559978,2560714,2561815,2562896,2565622,".to_owned(),
            },
            RefFlatRecord {
                gene_id: "SMIM12".to_owned(),
                transcript_name: "NM_001164824".to_owned(),
                seq_name: "chr1".to_owned(),
                strand: '-',
                trx_start: 34850361,
                trx_end: 34859045,
                coding_start: 34855698,
                coding_end: 34855977,
                num_exons: 3,
                exon_starts: "34850361,34856555,34858839,".to_owned(),
                exon_ends: "34855982,34856739,34859045,".to_owned(),
            },
            RefFlatRecord {
                gene_id: "SMIM12".to_owned(),
                transcript_name: "NM_001164825".to_owned(),
                seq_name: "chr1".to_owned(),
                strand: '-',
                trx_start: 34850361,
                trx_end: 34859737,
                coding_start: 34855698,
                coding_end: 34855977,
                num_exons: 2,
                exon_starts: "34850361,34859454,".to_owned(),
                exon_ends: "34855982,34859737,".to_owned(),
            },
            RefFlatRecord {
                gene_id: "SMIM12".to_owned(),
                transcript_name: "NM_138428".to_owned(),
                seq_name: "chr1".to_owned(),
                strand: '-',
                trx_start: 34850361,
                trx_end: 34859816,
                coding_start: 34855698,
                coding_end: 34855977,
                num_exons: 2,
                exon_starts: "34850361,34859676,".to_owned(),
                exon_ends: "34855982,34859816,".to_owned(),
            },
        ];

        let mut writer = Writer::from_writer(vec![]);
        for rec in recs.iter() {
            writer.write_record(rec).expect("a successful write");
        }
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS));
    }

    #[test]
    fn transcripts_mult_rows_mult_genes_with_cds() {
        let trxs = vec![
            TBuilder::new("chr1", 2556364, 2565622)
                .strand(Strand::Forward)
                .coords(vec![
                    (2556364, 2556733), (2557725, 2557834), (2558342, 2558468), (2559822, 2559978),
                    (2560623, 2560714), (2562864, 2562896), (2563147, 2565622)],
                    Some((2556664, 2562868)))
                .attribute("gene_id", "TNFRSF14").id("NM_001297605")
                .coding_incl_stop(true)
                .build().expect("a transcript"),
            TBuilder::new("chr1", 2556364, 2565622)
                .strand(Strand::Forward)
                .coords(vec![
                    (2556364, 2556733), (2557725, 2557834), (2558342, 2558468), (2559822, 2559978),
                    (2560623, 2560714), (2561672, 2561815), (2562864, 2562896),
                    (2563147, 2565622)], Some((2556664, 2563273)))
                .attribute("gene_id", "TNFRSF14").id("NM_003820")
                .coding_incl_stop(true)
                .build().expect("a transcript"),
            TBuilder::new("chr1", 34850361, 34859045)
                .strand(Strand::Reverse)
                .coords(vec![
                    (34850361, 34855982), (34856555, 34856739), (34858839, 34859045)],
                    Some((34855698, 34855977)))
                .attribute("gene_id", "SMIM12").id("NM_001164824")
                .coding_incl_stop(true)
                .build().expect("a transcript"),
            TBuilder::new("chr1", 34850361, 34859737)
                .strand(Strand::Reverse)
                .coords(vec![
                    (34850361, 34855982), (34859454, 34859737)], Some((34855698, 34855977)))
                .attribute("gene_id", "SMIM12").id("NM_001164825")
                .coding_incl_stop(true)
                .build().expect("a transcript"),
            TBuilder::new("chr1", 34850361, 34859816)
                .strand(Strand::Reverse)
                .coords(vec![
                    (34850361, 34855982), (34859676, 34859816)], Some((34855698, 34855977)))
                .attribute("gene_id", "SMIM12").id("NM_138428")
                .coding_incl_stop(true)
                .build().expect("a transcript"),
        ];

        let mut writer = Writer::from_writer(vec![]);
        for trx in trxs.iter() {
            writer.write_transcript(trx).expect("a successful write");
        }
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS));
    }

    #[test]
    fn genes_mult_rows_mult_genes_with_cds() {
        let mut trxs1 = Vec::new();
        trxs1.push((
            "NM_001297605".to_owned(),
            ((2556364, 2565622),
             vec![(2556364, 2556733), (2557725, 2557834), (2558342, 2558468), (2559822, 2559978),
                  (2560623, 2560714), (2562864, 2562896), (2563147, 2565622)],
             Some((2556664, 2562868)))));

        trxs1.push((
           "NM_003820".to_owned(),
           ((2556364, 2565622),
            vec![(2556364, 2556733), (2557725, 2557834), (2558342, 2558468), (2559822, 2559978),
                 (2560623, 2560714), (2561672, 2561815), (2562864, 2562896), (2563147, 2565622)],
            Some((2556664, 2563273)))));

        let mut trxs2 = Vec::new();
        trxs2.push((
           "NM_001164824".to_owned(),
           ((34850361, 34859045),
            vec![(34850361, 34855982), (34856555, 34856739), (34858839, 34859045)],
            Some((34855698, 34855977)))));

        trxs2.push((
           "NM_001164825".to_owned(),
           ((34850361, 34859737), vec![(34850361, 34855982), (34859454, 34859737)],
            Some((34855698, 34855977)))));

        trxs2.push((
           "NM_138428".to_owned(),
           ((34850361, 34859816), vec![(34850361, 34855982), (34859676, 34859816)],
            Some((34855698, 34855977)))));

        let gxs = vec![
            GBuilder::new("chr1", 2556364, 2565622)
                .strand(Strand::Forward)
                .id("TNFRSF14")
                .transcript_coords(trxs1)
                .transcript_coding_incl_stop(true)
                .build().expect("a gene"),
            GBuilder::new("chr1", 34850361, 34859816)
                .strand(Strand::Reverse)
                .id("SMIM12")
                .transcript_coords(trxs2)
                .transcript_coding_incl_stop(true)
                .build().expect("a gene"),
        ];

        let mut writer = Writer::from_writer(vec![]);
        for gx in gxs.iter() {
            writer.write_gene(gx).expect("a successful write");
        }
        assert_eq!(writer.inner.as_string(),
                   String::from_utf8_lossy(REFFLAT_MULT_ROWS_MULT_GENES_WITH_CDS));
    }
}
