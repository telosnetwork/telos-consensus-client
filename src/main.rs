mod block_reader;

fn main() {
    let reader = block_reader::FileBlockReader::new();
    let block_zero = reader.get_block(0);
    println!("block_zero: {:?}", block_zero);
}

#[cfg(test)]
mod tests {
    use crate::block_reader::FileBlockReader;

    #[test]
    fn block_reader() {
        let reader = FileBlockReader::new();
        let block_zero = reader.get_block(0);
        assert!(block_zero.is_some());
    }
}
