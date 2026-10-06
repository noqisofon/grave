/// 再現性のための小さな xorshift 乱数。
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// lo 以上 hi 未満の整数。
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        debug_assert!(lo < hi);
        lo + (self.next_u64() % (hi - lo) as u64) as i32
    }
}
