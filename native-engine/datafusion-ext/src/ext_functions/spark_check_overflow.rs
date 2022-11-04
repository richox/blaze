// Copyright 2022 The Blaze Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;
use datafusion::arrow::array::*;
use datafusion::common::ScalarValue;
use datafusion::common::Result;
use datafusion::physical_plan::ColumnarValue;

/// implements org.apache.spark.sql.catalyst.expressions.MakeDecimal
pub fn spark_check_overflow(args: &[ColumnarValue]) -> Result<ColumnarValue> {
    let to_precision = match &args[1] {
        &ColumnarValue::Scalar(ScalarValue::Int32(Some(precision))) => precision as u8,
        _ => unreachable!("check_overflow.precision is not int32 value"),
    };
    let to_scale = match &args[2] {
        &ColumnarValue::Scalar(ScalarValue::Int32(Some(scale))) => scale as u8,
        _ => unreachable!("check_overflow.scale is not int32 value"),
    };
    assert!(to_precision >= 1, "check_overflow: illegal precision: {}", to_precision);

    Ok(match &args[0] {
        ColumnarValue::Scalar(scalar) => match scalar {
            ScalarValue::Decimal128(Some(i128_val), precision, scale) => {
                ColumnarValue::Scalar(
                    ScalarValue::Decimal128(
                        change_precision_round_half_up(
                            *i128_val,
                            *precision,
                            *scale,
                            to_precision,
                            to_scale),
                        to_precision,
                        to_scale
                    ))
            }
            _ => {
                ColumnarValue::Scalar(ScalarValue::Decimal128(None, to_precision, to_scale))
            },
        },
        ColumnarValue::Array(array) => {
            let array = array.as_any().downcast_ref::<Decimal128Array>().unwrap();
            let mut output = Decimal128Builder::with_capacity(array.len());

            for v in array.into_iter() {
                match v {
                    Some(v) => {
                        output.append_option(
                            change_precision_round_half_up(
                                v,
                                array.precision(),
                                array.scale(),
                                to_precision,
                                to_scale));
                    }
                    None => output.append_null(),
                }
            }
            ColumnarValue::Array(Arc::new(output
                .finish()
                .with_precision_and_scale(to_precision, to_scale)?))
        }
    })
}

/// implements org.apache.spark.sql.types.Decimal.changePrecision
fn change_precision_round_half_up(
    mut i128_val: i128,
    precision: u8,
    scale: u8,
    to_precision: u8,
    to_scale: u8
) -> Option<i128> {
    let max_spark_precision = 38;

    if to_precision == precision && to_scale == scale {
        return Some(i128_val);
    }
    if to_scale < scale {
        // Easier case: we just need to divide our scale down
        let diff = scale - to_scale;
        let pow10diff = i128::pow(10, diff as u32);
        // % and / always round to 0
        let dropped_digits = i128_val % pow10diff;
        i128_val /= pow10diff;
        if dropped_digits.abs() * 2 >= pow10diff {
            i128_val += if dropped_digits < 0 {
                -1
            } else {
                1
            };
        }
    } else if to_scale > scale {
        // We might be able to multiply i128_val by a power of 10 and not overflow, but if not,
        // switch to using a BigDecimal
        let diff = to_scale - scale;
        // Multiplying i128_val by POW_10(diff) will still keep it below max_long_digits
        i128_val *= i128::pow(10, diff as u32);
    }

    // check whether the i128_val overflows s max precision supported in spark
    let p = i128::pow(10, u32::min(to_precision as u32, max_spark_precision));
    if i128_val <= -p || i128_val >= p {
        return None;
    }
    Some(i128_val)
}