use std::collections::HashMap;

pub struct Matcher<'a> {
    a: &'a [String],
    b: &'a [String],
    b2j: HashMap<&'a str, Vec<usize>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Equal,
    Replace,
    Delete,
    Insert,
}

impl<'a> Matcher<'a> {
    pub fn new(a: &'a [String], b: &'a [String]) -> Matcher<'a> {
        let mut b2j: HashMap<&str, Vec<usize>> = HashMap::new();
        for (j, x) in b.iter().enumerate() {
            b2j.entry(x.as_str()).or_default().push(j);
        }
        Matcher { a, b, b2j }
    }

    fn longest(&self, alo: usize, ahi: usize, blo: usize, bhi: usize) -> (usize, usize, usize) {
        let (mut bi, mut bj, mut size) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut next = HashMap::new();
            for &j in self.b2j.get(self.a[i].as_str()).map(Vec::as_slice).unwrap_or(&[]) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                next.insert(j, k);
                if k > size {
                    (bi, bj, size) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = next;
        }
        while bi > alo && bj > blo && self.a[bi - 1] == self.b[bj - 1] {
            (bi, bj, size) = (bi - 1, bj - 1, size + 1);
        }
        while bi + size < ahi && bj + size < bhi && self.a[bi + size] == self.b[bj + size] {
            size += 1;
        }
        (bi, bj, size)
    }

    pub fn blocks(&self) -> Vec<(usize, usize, usize)> {
        let (la, lb) = (self.a.len(), self.b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut found = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest(alo, ahi, blo, bhi);
            if k > 0 {
                found.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        found.sort_unstable();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        let mut out = Vec::new();
        for (i2, j2, k2) in found {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    out.push((i1, j1, k1));
                }
                (i1, j1, k1) = (i2, j2, k2);
            }
        }
        if k1 > 0 {
            out.push((i1, j1, k1));
        }
        out.push((la, lb, 0));
        out
    }

    pub fn matches(&self) -> usize {
        self.blocks().iter().map(|b| b.2).sum()
    }

    pub fn opcodes(&self) -> Vec<(Op, usize, usize, usize, usize)> {
        let (mut i, mut j) = (0, 0);
        let mut out = Vec::new();
        for (ai, bj, size) in self.blocks() {
            let op = match (i < ai, j < bj) {
                (true, true) => Some(Op::Replace),
                (true, false) => Some(Op::Delete),
                (false, true) => Some(Op::Insert),
                _ => None,
            };
            if let Some(op) = op {
                out.push((op, i, ai, j, bj));
            }
            (i, j) = (ai + size, bj + size);
            if size > 0 {
                out.push((Op::Equal, ai, i, bj, j));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split(' ').map(String::from).collect()
    }

    #[test]
    fn blocks_and_opcodes_match_python_difflib() {
        let (a, b) = (v("q w e r t y"), v("q x e r z y"));
        let m = Matcher::new(&a, &b);
        assert_eq!(m.blocks(), vec![(0, 0, 1), (2, 2, 2), (5, 5, 1), (6, 6, 0)]);
        assert_eq!(
            m.opcodes(),
            vec![
                (Op::Equal, 0, 1, 0, 1),
                (Op::Replace, 1, 2, 1, 2),
                (Op::Equal, 2, 4, 2, 4),
                (Op::Replace, 4, 5, 4, 5),
                (Op::Equal, 5, 6, 5, 6)
            ]
        );
        let (a, b) = (v("a b c a b"), v("b a b c"));
        assert_eq!(Matcher::new(&a, &b).blocks(), vec![(0, 1, 3), (5, 4, 0)]);
        let (a, b) = (v("x y"), v("x y z"));
        assert_eq!(Matcher::new(&a, &b).opcodes(), vec![(Op::Equal, 0, 2, 0, 2), (Op::Insert, 2, 2, 2, 3)]);
    }
}
