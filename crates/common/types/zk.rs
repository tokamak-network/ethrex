use rkyv::{Archive, Deserialize as RDeserialize, Serialize as RSerialize};
use serde::{Deserialize, Serialize};
use ethrex_rlp::{
    decode::RLPDecode,
    encode::RLPEncode,
    error::RLPDecodeError,
    structs::{Decoder, Encoder},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, RSerialize, RDeserialize, Archive)]
pub struct BlockProof {
    pub proof: Vec<u8>,
}

impl RLPEncode for BlockProof {
    fn encode(&self, buf: &mut dyn bytes::BufMut) {
        Encoder::new(buf)
            .encode_field(&self.proof)
            .finish();
    }
}

impl RLPDecode for BlockProof {
    fn decode_unfinished(rlp: &[u8]) -> Result<(Self, &[u8]), RLPDecodeError> {
        let decoder = Decoder::new(rlp)?;
        let (proof, decoder) = decoder.decode_field("proof")?;
        Ok((BlockProof { proof }, decoder.finish()?))
    }
}
