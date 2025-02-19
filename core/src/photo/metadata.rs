// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

use super::model::Orientation;
use super::Metadata;
use anyhow::*;
use chrono::prelude::*;
use chrono::{DateTime, FixedOffset};
use exif;
use exif::Exif;
use std::fs;
use std::io::BufReader;
use std::path::Path;
use std::result::Result::Ok;

/// This version number should be incremented each time metadata scanning has
/// a bug fix or feature addition that changes the metadata produced.
/// Each photo will be saved with a metadata scan version which will allow for
/// easy selection of photos when there metadata can be updated.

pub const VERSION: u32 = 2;

/// Extract EXIF metadata from file
pub fn from_path(path: &Path) -> Result<Metadata> {
    let file = fs::File::open(path)?;
    let file = &mut BufReader::new(file);
    let exif_data = {
        match exif::Reader::new().read_from_container(file) {
            Ok(exif) => exif,
            Err(_) => {
                // Assume this error is when there is no EXIF data.
                return Ok(Metadata::default());
            }
        }
    };

    let mut metadata = from_exif(exif_data)?;

    // FIXME what is a better way of doing this?
    //
    // libheif applies the orientation transformation when loading the image,
    // so we must not re-apply the transformation when displaying the image, otherwise
    // we will double transform and show the image incorrectly.
    //
    // To fix that, I'm removing the orientation metadata if the file extension is 'heic'...
    // but it doesn't seem right.
    //
    // Note that this means from_file(...) and from_raw(...) will
    // return inconsistent metadata... again :-(

    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());

    if ext.is_some_and(|x| x == "heic") {
        metadata.orientation = None;
    }

    Ok(metadata)
}

/// Extract EXIF metadata from raw buffer
pub fn from_raw(data: Vec<u8>) -> Result<Metadata> {
    let exif_data = {
        match exif::Reader::new().read_raw(data) {
            Ok(exif) => exif,
            Err(_) => {
                // Assume this error is when there is no EXIF data.
                return Ok(Metadata::default());
            }
        }
    };

    from_exif(exif_data)
}

fn from_exif(exif_data: Exif) -> Result<Metadata> {
    fn parse_date_time(
        date_time_field: Option<&exif::Field>,
        time_offset_field: Option<&exif::Field>,
    ) -> Option<DateTime<FixedOffset>> {
        let date_time_field = date_time_field?;

        let mut date_time = match date_time_field.value {
            exif::Value::Ascii(ref vec) => exif::DateTime::from_ascii(&vec[0]).ok(),
            _ => None,
        }?;

        if let Some(field) = time_offset_field {
            if let exif::Value::Ascii(ref vec) = field.value {
                let _ = date_time.parse_offset(&vec[0]);
            }
        }

        let offset = date_time.offset.unwrap_or(0); // offset in minutes
        let offset = FixedOffset::east_opt((offset as i32) * 60)?;

        let date = NaiveDate::from_ymd_opt(
            date_time.year.into(),
            date_time.month.into(),
            date_time.day.into(),
        )?;

        let time = NaiveTime::from_hms_opt(
            date_time.hour.into(),
            date_time.minute.into(),
            date_time.second.into(),
        )?;

        let naive_date_time = date.and_time(time);
        Some(offset.from_utc_datetime(&naive_date_time))
    }

    let mut metadata = Metadata::default();

    metadata.created_at = parse_date_time(
        exif_data.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY),
        exif_data.get_field(exif::Tag::OffsetTimeOriginal, exif::In::PRIMARY),
    );
    metadata.modified_at = parse_date_time(
        exif_data.get_field(exif::Tag::DateTime, exif::In::PRIMARY),
        exif_data.get_field(exif::Tag::OffsetTime, exif::In::PRIMARY),
    );

    metadata.lens_model = exif_data
        .get_field(exif::Tag::LensModel, exif::In::PRIMARY)
        .map(|e| e.display_value().to_string());

    // How to orient and flip the image.
    // Note that libheif will automatically apply the transformations when loading the image
    // so must be aware of file format before transforming to avoid a double transformation.
    metadata.orientation = exif_data
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|e| e.value.get_uint(0))
        .map(|e| Orientation::from(e));

    metadata.content_id = ios_content_id(&exif_data);

    Ok(metadata)
}

/// Parse content ID from the Apple maker note
fn ios_content_id(exif_data: &Exif) -> Option<String> {
    let maker_note = exif_data.get_field(exif::Tag::MakerNote, exif::In::PRIMARY)?;
    let exif::Value::Undefined(ref raw, _offset) = maker_note.value else {
        return None;
    };

    if !raw.starts_with(b"Apple iOS\0") {
        return None;
    }

    // We have an Apple maker note which contains EXIF tags, but they aren't quite in the right format
    // for the exif-rs library to parse.
    //
    // EXIF data starts at byte 12 with a byte order mark, but doesn't include the magic 0x2a.
    //
    // To fix this we rewrite the begining of the buffer to do the following:
    // 1. Start with a byte order mark (0x4d4d);
    // 2. Have the Douglas constant (0x002a).
    // 3. Have the byte offset point to the first piece of data (14)

    let mut buf: Vec<u8> = Vec::new();
    buf.extend_from_slice(raw);

    buf[0] = 0x4d;
    buf[1] = 0x4d;
    buf[2] = 0;
    buf[3] = 0x2a; // the Douglas constant
    buf[4] = 0;
    buf[5] = 0;
    buf[6] = 0;
    buf[7] = 14;

    let exif_data = exif::Reader::new().read_raw(buf).ok()?;

    let content_id =
        exif_data.get_field(exif::Tag(exif::Context::Tiff, 0x11), exif::In::PRIMARY)?;

    let content_id = match content_id.value {
        exif::Value::Ascii(ref vecs) => {
            let mut bytes = Vec::with_capacity(vecs[0].len());
            bytes.extend_from_slice(&vecs[0]);
            String::from_utf8(bytes).ok()
        }
        _ => None,
    };

    content_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ios_content_id() {
        let dir = env!("CARGO_MANIFEST_DIR");
        let file = Path::new(dir).join("resources/test/Dandelion.jpg");
        let file = fs::File::open(file).unwrap();
        let file = &mut BufReader::new(file);

        let exif_data = exif::Reader::new().read_from_container(file).ok().unwrap();
        let content_id = ios_content_id(&exif_data);

        assert_eq!(
            Some("5D3FF377-55D1-4BFF-A4FF-56B2298FC6C2".to_string()),
            content_id
        );
    }
}
