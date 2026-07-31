use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use anyhow::{bail, Result};

pub fn generate_hints(alphabet: &[char], count: usize) -> Result<Vec<String>> {
    validate_alphabet(alphabet, count)?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if count <= alphabet.len() {
        return Ok(alphabet.iter().take(count).map(char::to_string).collect());
    }

    let arity = alphabet.len();
    let dummy_count = (arity - 1 - ((count - 1) % (arity - 1))) % (arity - 1);
    let mut nodes = Vec::<Node>::new();
    let mut heap = BinaryHeap::<Reverse<(usize, usize, usize)>>::new();
    let mut serial = 0;
    for _ in 0..dummy_count {
        push_leaf(&mut nodes, &mut heap, &mut serial, false, 0);
    }
    for _ in 0..count {
        push_leaf(&mut nodes, &mut heap, &mut serial, true, 1);
    }

    while heap.len() > 1 {
        let mut children = Vec::with_capacity(arity);
        let mut weight = 0;
        for _ in 0..arity {
            let Reverse((child_weight, _, child)) = heap.pop().expect("valid Huffman arity");
            weight += child_weight;
            children.push(child);
        }
        let index = nodes.len();
        nodes.push(Node {
            real_leaf: false,
            children,
        });
        heap.push(Reverse((weight, serial, index)));
        serial += 1;
    }

    let Reverse((_, _, root)) = heap.pop().expect("non-empty hint tree");
    let mut hints = Vec::with_capacity(count);
    collect_hints(root, String::new(), alphabet, &nodes, &mut hints);
    let rank = alphabet
        .iter()
        .enumerate()
        .map(|(index, key)| (*key, index))
        .collect::<HashMap<_, _>>();
    hints.sort_by(|left, right| {
        left.len().cmp(&right.len()).then_with(|| {
            left.chars()
                .map(|key| rank[&key])
                .cmp(right.chars().map(|key| rank[&key]))
        })
    });
    Ok(hints)
}

struct Node {
    real_leaf: bool,
    children: Vec<usize>,
}

fn push_leaf(
    nodes: &mut Vec<Node>,
    heap: &mut BinaryHeap<Reverse<(usize, usize, usize)>>,
    serial: &mut usize,
    real_leaf: bool,
    weight: usize,
) {
    let index = nodes.len();
    nodes.push(Node {
        real_leaf,
        children: Vec::new(),
    });
    heap.push(Reverse((weight, *serial, index)));
    *serial += 1;
}

fn collect_hints(
    node: usize,
    prefix: String,
    alphabet: &[char],
    nodes: &[Node],
    output: &mut Vec<String>,
) {
    if nodes[node].children.is_empty() {
        if nodes[node].real_leaf {
            output.push(prefix);
        }
        return;
    }
    for (index, child) in nodes[node].children.iter().enumerate() {
        let mut child_prefix = prefix.clone();
        child_prefix.push(alphabet[index]);
        collect_hints(*child, child_prefix, alphabet, nodes, output);
    }
}

fn validate_alphabet(alphabet: &[char], count: usize) -> Result<()> {
    if count == 0 {
        return Ok(());
    }
    if alphabet.len() < 2 {
        bail!("hint alphabet must contain at least two keys");
    }
    if alphabet.iter().copied().collect::<HashSet<_>>().len() != alphabet.len() {
        bail!("hint alphabet keys must be unique");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn small_sets_use_one_key_in_alphabet_order() {
        assert_eq!(
            generate_hints(&['a', 's', 'd'], 3).unwrap(),
            vec!["a", "s", "d"]
        );
    }

    #[test]
    fn larger_sets_are_unique_prefix_free_and_short() {
        let hints = generate_hints(&['a', 's', 'd'], 5).unwrap();
        let unique = hints.iter().collect::<HashSet<_>>();

        assert_eq!(unique.len(), 5);
        assert!(hints.iter().any(|hint| hint.len() == 1));
        assert!(hints.iter().any(|hint| hint.len() == 2));
        for left in &hints {
            for right in &hints {
                if left != right {
                    assert!(!right.starts_with(left));
                }
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(
            generate_hints(&['a', 's', 'd', 'f'], 20).unwrap(),
            generate_hints(&['a', 's', 'd', 'f'], 20).unwrap()
        );
    }

    #[test]
    fn invalid_alphabets_fail_and_zero_targets_are_empty() {
        assert!(generate_hints(&['a'], 2).is_err());
        assert!(generate_hints(&['a', 'a'], 2).is_err());
        assert!(generate_hints(&['a', 's'], 0).unwrap().is_empty());
    }
}
