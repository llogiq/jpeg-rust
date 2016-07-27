use jpeg::huffman;
use ::transform;

// TODO: move this?
fn u8s_to_u16(bytes: &[u8]) -> u16 {
    let msb = bytes[0] as u16;
    let lsb = bytes[1] as u16;
    (msb << 8) + lsb
}


#[derive(Debug)]
pub enum JFIFUnits {
    NoUnits,
    DotsPerInch,
    DotsPerCm,
}

impl JFIFUnits {
    pub fn from_u8(byte: u8) -> Result<JFIFUnits, String> {
        Ok(match byte {
            1 => JFIFUnits::NoUnits,
            2 => JFIFUnits::DotsPerInch,
            3 => JFIFUnits::DotsPerCm,
            _ => return Err(format!("Illegal unit byte: {}", byte)),
        })
    }
}

#[derive(Debug)]
#[allow(non_camel_case_types)]
pub enum JFIFVersion {
    V1_01,
}

impl JFIFVersion {
    pub fn from_bytes(msb: u8, lsb: u8) -> Result<JFIFVersion, String> {
        Ok(match (msb, lsb) {
            (1, 1) => JFIFVersion::V1_01,
            _ => return Err(format!("Illegal version: ({}, {})", msb, lsb)),
        })
    }
}

type JPEGDimensions = (u16, u16);
type ThumbnailDimensions = (u8, u8);

#[derive(Debug)]
pub struct JFIFImage {
    version: JFIFVersion,
    units: JFIFUnits,
    dimensions: JPEGDimensions,
    thumbnail_dimensions: ThumbnailDimensions,
    comment: Option<String>,
    huffman_ac_tables: [Option<huffman::Table>; 4],
    huffman_dc_tables: [Option<huffman::Table>; 4],
    quantization_tables: [Option<Vec<u8>>; 4],
    // TODO: multiple frames ?
    frame_header: Option<FrameHeader>,

    // tmp
    data_index: usize,
    raw_data: Vec<u8>, // TOOD: add all options, such as progressive/sequential, etc.
}

#[derive(Debug)]
struct FrameHeader {
    sample_precision: u8,
    num_lines: u16,
    samples_per_line: u16,
    image_components: u8,
    frame_components: Vec<FrameComponentHeader>,
}

impl FrameHeader {
    fn component_header(&self, id: u8) -> Option<&FrameComponentHeader> {
        self.frame_components.iter().find(|c| c.component_id == id)
    }
}

#[derive(Debug)]
struct FrameComponentHeader {
    component_id: u8,
    horizontal_sampling_factor: u8,
    vertical_sampling_factor: u8,
    quantization_selector: u8,
}

#[allow(unused_variables)]
impl JFIFImage {
    pub fn parse(vec: Vec<u8>) -> Result<JFIFImage, String> {
        // you can identify a JFIF file by looking for the following sequence:
        //
        //      X'FF', SOI, X'FF', APP0, <2 bytes to be skipped>, "JFIF", X'00'.
        if vec.len() < 11 {
            return Err("input is too short".to_string());
        }
        let SOI = 0xd8;
        let APP0 = 0xe0;
        if vec[0] != 0xff || vec[1] != SOI || vec[2] != 0xff || vec[3] != APP0 ||
           vec[6] != 'J' as u8 || vec[7] != 'F' as u8 || vec[8] != 'I' as u8 ||
           vec[9] != 'F' as u8 || vec[10] != 0x00 {
            return Err("Header mismatch".to_string());
        }
        let version = try!(JFIFVersion::from_bytes(vec[11], vec[12]));

        let units = try!(JFIFUnits::from_u8(vec[13]));
        let x_density = u8s_to_u16(&vec[14..16]);
        let y_density = u8s_to_u16(&vec[16..18]);
        let thumbnail_dimensions = (vec[18], vec[19]);

        // TODO: thumbnail data?
        // let n = thumbnail_dimensions.0 as usize * thumbnail_dimensions.1 as usize;

        let mut jfif_image = JFIFImage {
            version: version,
            units: units,
            dimensions: (x_density, y_density),
            thumbnail_dimensions: thumbnail_dimensions,
            comment: None,
            huffman_ac_tables: [None, None, None, None],
            huffman_dc_tables: [None, None, None, None],
            quantization_tables: [None, None, None, None],
            frame_header: None,

            data_index: 0,
            raw_data: Vec::new(),
        };

        let bytes_to_len = |a: u8, b: u8| ((a as usize) << 8) + b as usize - 2;

        let mut i = 20;
        loop {
            // All segments have a 2 byte length
            // right after the marker code
            let data_length = bytes_to_len(vec[i + 2], vec[i + 3]);
            match (vec[i], vec[i + 1]) {
                (0xff, 0xfe) => {
                    // Comment
                    use std::str;
                    let comment: String = match str::from_utf8(&vec[i + 4..i + 4 + data_length]) {
                        Ok(s) => s.to_string(),
                        Err(e) => {
                            println!("{}", e);
                            "".to_string()
                        }
                    };
                    // println!("found comment '{}'", comment);
                }
                (0xff, 0xdb) => {
                    // Quantization tables
                    // JPEG B.2.4.1

                    let precision = (vec[i + 4] & 0xf0) >> 4;
                    let identifier = vec[i + 4] & 0x0f;
                    let quant_values = &vec[i + 5..i + 4 + data_length];
                    // TODO: we probably dont need to copy and collect here.
                    // Would rather have a slice in quant_tables, with a
                    // lifetime the same as jfif_image (?)
                    let table = quant_values.iter()
                        .map(|u| *u)
                        .collect();
                    jfif_image.quantization_tables[identifier as usize] = Some(table);
                }
                (0xff, 0xc0) => {
                    // Baseline DCT
                    // JPEG B.2.2
                    let sample_precision = vec[i + 4];
                    let num_lines = u8s_to_u16(&vec[i + 5..]);
                    let samples_per_line = u8s_to_u16(&vec[i + 7..]);
                    let image_components = vec[i + 9];
                    if image_components != 1 {
                        panic!("FIXME! 'Baseline DCT");
                    }
                    let component_id = vec[i + 10];
                    let horizontal_sampling_factor = (vec[i + 11] & 0xf0) >> 4;
                    let vertical_sampling_factor = vec[i + 11] & 0x0f;
                    let quantization_selector = vec[i + 12];

                    let frame_component = FrameComponentHeader {
                        component_id: component_id,
                        horizontal_sampling_factor: horizontal_sampling_factor,
                        vertical_sampling_factor: vertical_sampling_factor,
                        quantization_selector: quantization_selector,
                    };
                    let frame_header = FrameHeader {
                        sample_precision: sample_precision,
                        num_lines: num_lines,
                        samples_per_line: samples_per_line,
                        image_components: image_components,
                        frame_components: vec![frame_component],
                    };
                    jfif_image.frame_header = Some(frame_header)
                }
                (0xff, 0xc4) => {
                    // Define Huffman table
                    // JPEG B.2.4.2
                    // DC = 0, AC = 1
                    let table_class = (vec[i + 4] & 0xf0) >> 4;
                    let table_dest_id = vec[i + 4] & 0x0f;

                    // There are size_area[i] number of codes of length i + 1.
                    let size_area: &[u8] = &vec[i + 5..i + 5 + 16];
                    // Code i has value data_area[i]
                    let data_area: &[u8] = &vec[i + 5 + 16..i + 4 + data_length];
                    let huffman_table = huffman::Table::from_size_data_tables(size_area, data_area);
                    let ind = table_dest_id as usize;
                    if table_class == 0 {
                        jfif_image.huffman_dc_tables[ind] = Some(huffman_table);
                    } else {
                        jfif_image.huffman_ac_tables[ind] = Some(huffman_table);
                    }
                }
                (0xff, 0xda) => {
                    // Start of Scan
                    // JPEG B.2.3
                    let num_components = vec[i + 4];
                    if num_components != 1 {
                        panic!("FIXME! I took the easy way!")
                    }
                    let scan_component_selector = vec[i + 5];
                    let dc_table_id = (vec[i + 6] & 0xf0) >> 4;
                    let ac_table_id = vec[i + 6] & 0x0f;
                    i += 2 * num_components as usize;

                    let start_spectral_section = vec[i + 5];
                    let end_spectral_section = vec[i + 6];
                    let al_ah = vec[i + 7];
                    // `i` is now at the head of the data.
                    i += 8;

                    // After the scan header is parsed, we start to read data.
                    // See Figure B.2 in B.2.1
                    //
                    // But first, we get all the tables.
                    // NOTE: this assumes no restart!
                    //       Check if it is handled: `(0xff, 0xdd)`


                    let ac_table = jfif_image.huffman_ac_tables[ac_table_id as usize]
                        .as_ref()
                        .expect("Did not find AC table");

                    let dc_table = jfif_image.huffman_dc_tables[dc_table_id as usize]
                        .as_ref()
                        .expect("Did not find DC table");

                    // TODO: Should find a better way of doing this,
                    //       as either `None` is a bad error, from which
                    //       recovery is not an option?
                    let quant_table_id = match jfif_image.frame_header {
                        Some(ref frame_header) => {
                            match frame_header.component_header(scan_component_selector) {
                                Some(frame_component_header) => {
                                    frame_component_header.quantization_selector
                                }
                                None => {
                                    panic!(format!("Could not find frame component for \
                                                     scan_component_selector {}",
                                                   scan_component_selector))
                                }
                            }
                        }
                        None => panic!("jfif_image has no frame_header!"),
                    };

                    let ref quant_table = jfif_image.quantization_tables[quant_table_id as usize]
                        .as_ref()
                        .expect(&format!("Did not find quantization table of id {}",
                                         quant_table_id));


                    let mut image_blocks = Vec::<Vec<u8>>::new();
                    let n_blocks_x = (jfif_image.dimensions.0 + 7) / 8; // round up
                    let n_blocks_y = (jfif_image.dimensions.1 + 7) / 8; // round up
                    for _ in 0..(n_blocks_x * n_blocks_y) {
                        let (decoded, num_read) = huffman::decode(ac_table, dc_table, &vec[i..]);
                        if decoded.len() != 64 {
                            panic!("length should be 64!!")
                        }

                        let dequantized: Vec<i16> = quant_table.iter()
                            .zip(decoded.iter())
                            .map(|n| {
                                println!("{:?}", n);
                                n
                            })
                            .map(|(&q, &n)| (q as i16) * n)
                            .collect();

                        let dequantized_f32 = dequantized.iter().map(|&i| i as f32).collect();
                        let spatial =
                            transform::discrete_cosine_transform_inverse(&dequantized_f32);
                        // TODO: u8 is probably not what we want?
                        let rounded_and_shifted = spatial.iter()
                            .map(|&f| (f.round() + 128f32) as u8);

                        image_blocks.push(rounded_and_shifted.collect());

                        i += num_read as usize;
                    }
                }
                (0xff, 0xdd) => {
                    // Restart Interval Definition
                    // JPEG B.2.4.4
                    // TODO: support this
                    panic!("got to restart interval def")
                }
                _ => {
                    println!("\n\nUnhandled byte marker: {:02x} {:02x}",
                             vec[i],
                             vec[i + 1]);
                    println!("len={}", data_length);
                    print_vector(vec.iter().skip(i));
                    break;
                }
            }
            i += 4 + data_length;
        }
        panic!("WHAT TO DO");
        // Ok(jfif_image)
    }
}

// TODO: Remove (or move?)
use std::fmt::LowerHex;
fn print_vector<I>(iter: I)
    where I: Iterator,
          I::Item: LowerHex
{
    let mut i = 0;
    for byte in iter.take(128) {
        i += 1;
        print!("{:02x} ", byte);
        if i % 16 == 0 && i != 0 {
            print!("\n");
        }
    }
    if i % 16 != 0 || i == 0 {
        print!("\n");
    }
}
use std::fmt::Display;
fn print_vector_dec<I>(iter: I)
    where I: Iterator,
          I::Item: Display
{
    let mut i = 0;
    for byte in iter.take(64) {
        i += 1;
        print!("{:3} ", byte);
        if i % 8 == 0 && i != 0 {
            print!("\n");
        }
    }
    if i % 8 != 0 || i == 0 {
        print!("\n");
    }
}

/// Turn a vector representing a Matrix into 'zigzag' order.
///
/// ```
///  0  1  2  3
///  4  5  6  7
///  8  9 10 11
/// 12 13 14 15
///
/// becomes
///
///  0  1  5  6
///  2  4  7 12
///  3  8 11 13
///  9 10 14 15
/// ```
///
fn zigzag<T>(vec: Vec<T>) -> Vec<T>
    where T: Copy
{
    if vec.len() != 64 {
        panic!("I took a shortcut in zigzag()! Please implement me properly :) (len={})",
               vec.len());
    }
    // hardcode dis shit lol
    let indices = [0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48,
                   41, 34, 27, 20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22,
                   15, 23, 30, 37, 44, 51, 58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55,
                   62, 53];
    let mut res = Vec::with_capacity(64);
    for &i in indices.iter() {
        res.push(vec[i]);
    }
    res
}
