//! A minimal writer of ONNX models: SlopShop builds the graphs of the generative models it runs
//! itself (no exporter, no Python), the weights staying outside the graph. Only what the graphs
//! need is encoded: nodes with their attributes, typed inputs and outputs, small constants and
//! initializers whose data ONNX Runtime is given separately (external data, by offset into one
//! named buffer). The encoding is protobuf's wire format, written by hand (`onnx.proto`, IR 8).

/// Element types (`TensorProto.DataType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    F32 = 1,
    U8 = 2,
    I8 = 3,
    I64 = 7,
    F16 = 10,
}

impl DataType {
    pub fn size(self) -> usize {
        match self {
            DataType::F32 => 4,
            DataType::U8 | DataType::I8 => 1,
            DataType::I64 => 8,
            DataType::F16 => 2,
        }
    }
}

/// A dimension of an input or output: fixed, or named (dynamic).
#[derive(Debug, Clone, Copy)]
pub enum Dim {
    Fixed(i64),
    Named(&'static str),
}

/// A node attribute.
#[derive(Debug, Clone)]
pub enum Attr {
    Int(&'static str, i64),
    Float(&'static str, f32),
    Ints(&'static str, Vec<i64>),
    Str(&'static str, &'static str),
}

/// A graph under construction. Value names are generated (`v0`, `v1`…) unless given.
#[derive(Debug, Default)]
pub struct Graph {
    nodes: Vec<u8>,
    initializers: Vec<u8>,
    inputs: Vec<u8>,
    outputs: Vec<u8>,
    next: usize,
    /// Whether a `com.microsoft` operator is used (its opset is then imported).
    microsoft: bool,
}

impl Graph {
    pub fn new() -> Self {
        Self::default()
    }

    fn fresh(&mut self) -> String {
        self.next += 1;
        format!("v{}", self.next - 1)
    }

    /// Declares a graph input.
    pub fn input(&mut self, name: &str, dtype: DataType, dims: &[Dim]) -> String {
        value_info(&mut self.inputs, 11, name, dtype, dims);
        name.to_string()
    }

    /// Declares `value` a graph output named `name` (through an `Identity` when they differ).
    pub fn output(&mut self, value: &str, name: &str, dtype: DataType, dims: &[Dim]) {
        if value != name {
            self.node_named("Identity", &[value], &[name], &[]);
        }
        value_info(&mut self.outputs, 12, name, dtype, dims);
    }

    /// Adds a node with one output and returns that output's name.
    pub fn node(&mut self, op: &str, inputs: &[&str], attrs: &[Attr]) -> String {
        let out = self.fresh();
        self.node_named(op, inputs, &[&out], attrs);
        out
    }

    /// Adds a node with `count` outputs.
    pub fn node_n(
        &mut self,
        op: &str,
        inputs: &[&str],
        count: usize,
        attrs: &[Attr],
    ) -> Vec<String> {
        let outs: Vec<String> = (0..count).map(|_| self.fresh()).collect();
        let names: Vec<&str> = outs.iter().map(String::as_str).collect();
        self.node_named(op, inputs, &names, attrs);
        outs
    }

    /// Adds a node of the `com.microsoft` domain (ONNX Runtime's contrib operators).
    pub fn node_microsoft(&mut self, op: &str, inputs: &[&str], attrs: &[Attr]) -> String {
        self.microsoft = true;
        let out = self.fresh();
        let mut node = Vec::new();
        node_body(&mut node, op, inputs, &[&out], attrs);
        bytes(&mut node, 7, b"com.microsoft");
        bytes(&mut self.nodes, 1, &node);
        out
    }

    fn node_named(&mut self, op: &str, inputs: &[&str], outputs: &[&str], attrs: &[Attr]) {
        let mut node = Vec::new();
        node_body(&mut node, op, inputs, outputs, attrs);
        bytes(&mut self.nodes, 1, &node);
    }

    /// A constant held in the graph (small: shapes, axes, scalars).
    pub fn constant(&mut self, dtype: DataType, dims: &[i64], raw: &[u8]) -> String {
        let name = self.fresh();
        let mut t = Vec::new();
        tensor_header(&mut t, &name, dtype, dims);
        bytes(&mut t, 9, raw);
        bytes(&mut self.initializers, 5, &t);
        name
    }

    pub fn ints(&mut self, values: &[i64]) -> String {
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.constant(DataType::I64, &[values.len() as i64], &raw)
    }

    pub fn scalar(&mut self, value: f32) -> String {
        self.constant(DataType::F32, &[], &value.to_le_bytes())
    }

    pub fn floats(&mut self, values: &[f32]) -> String {
        let raw: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.constant(DataType::F32, &[values.len() as i64], &raw)
    }

    /// An initializer whose data is `length` bytes at `offset` in the external buffer `location`.
    pub fn external(
        &mut self,
        name: &str,
        dtype: DataType,
        dims: &[i64],
        location: &str,
        offset: usize,
        length: usize,
    ) -> String {
        let mut t = Vec::new();
        tensor_header(&mut t, name, dtype, dims);
        for (key, value) in [
            ("location", location.to_string()),
            ("offset", offset.to_string()),
            ("length", length.to_string()),
        ] {
            let mut entry = Vec::new();
            bytes(&mut entry, 1, key.as_bytes());
            bytes(&mut entry, 2, value.as_bytes());
            bytes(&mut t, 13, &entry);
        }
        int(&mut t, 14, 1); // data_location: EXTERNAL
        bytes(&mut self.initializers, 5, &t);
        name.to_string()
    }

    /// The serialized `ModelProto`.
    pub fn model(&self, name: &str, opset: i64) -> Vec<u8> {
        let mut graph = Vec::new();
        graph.extend_from_slice(&self.nodes);
        bytes(&mut graph, 2, name.as_bytes());
        graph.extend_from_slice(&self.initializers);
        graph.extend_from_slice(&self.inputs);
        graph.extend_from_slice(&self.outputs);

        let mut model = Vec::new();
        int(&mut model, 1, 8); // ir_version
        bytes(&mut model, 2, b"slopshop");
        let mut domains = vec![("", opset)];
        if self.microsoft {
            domains.push(("com.microsoft", 1));
        }
        for (domain, version) in domains {
            let mut set = Vec::new();
            bytes(&mut set, 1, domain.as_bytes());
            int(&mut set, 2, version);
            bytes(&mut model, 8, &set);
        }
        bytes(&mut model, 7, &graph);
        model
    }
}

fn node_body(out: &mut Vec<u8>, op: &str, inputs: &[&str], outputs: &[&str], attrs: &[Attr]) {
    for i in inputs {
        bytes(out, 1, i.as_bytes());
    }
    for o in outputs {
        bytes(out, 2, o.as_bytes());
    }
    if let Some(first) = outputs.first() {
        bytes(out, 3, first.as_bytes()); // the node named after its first output
    }
    bytes(out, 4, op.as_bytes());
    for a in attrs {
        let mut attr = Vec::new();
        match a {
            Attr::Int(name, v) => {
                bytes(&mut attr, 1, name.as_bytes());
                int(&mut attr, 3, *v);
                int(&mut attr, 20, 2);
            }
            Attr::Float(name, v) => {
                bytes(&mut attr, 1, name.as_bytes());
                key(&mut attr, 2, 5);
                attr.extend_from_slice(&v.to_le_bytes());
                int(&mut attr, 20, 1);
            }
            Attr::Ints(name, vs) => {
                bytes(&mut attr, 1, name.as_bytes());
                for v in vs {
                    int(&mut attr, 8, *v);
                }
                int(&mut attr, 20, 7);
            }
            Attr::Str(name, s) => {
                bytes(&mut attr, 1, name.as_bytes());
                bytes(&mut attr, 4, s.as_bytes());
                int(&mut attr, 20, 3);
            }
        }
        bytes(out, 5, &attr);
    }
}

fn tensor_header(out: &mut Vec<u8>, name: &str, dtype: DataType, dims: &[i64]) {
    for d in dims {
        int(out, 1, *d);
    }
    int(out, 2, dtype as i64);
    bytes(out, 8, name.as_bytes());
}

fn value_info(out: &mut Vec<u8>, field: u32, name: &str, dtype: DataType, dims: &[Dim]) {
    let mut shape = Vec::new();
    for d in dims {
        let mut dim = Vec::new();
        match d {
            Dim::Fixed(v) => int(&mut dim, 1, *v),
            Dim::Named(n) => bytes(&mut dim, 2, n.as_bytes()),
        }
        bytes(&mut shape, 1, &dim);
    }
    let mut tensor = Vec::new();
    int(&mut tensor, 1, dtype as i64);
    bytes(&mut tensor, 2, &shape);
    let mut ty = Vec::new();
    bytes(&mut ty, 1, &tensor);
    let mut info = Vec::new();
    bytes(&mut info, 1, name.as_bytes());
    bytes(&mut info, 2, &ty);
    bytes(out, field, &info);
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn key(out: &mut Vec<u8>, field: u32, wire: u8) {
    varint(out, (u64::from(field) << 3) | u64::from(wire));
}

fn int(out: &mut Vec<u8>, field: u32, v: i64) {
    key(out, field, 0);
    varint(out, v as u64);
}

fn bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    key(out, field, 2);
    varint(out, data.len() as u64);
    out.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_and_negative_ints_follow_protobuf() {
        let mut out = Vec::new();
        varint(&mut out, 300);
        assert_eq!(out, [0xac, 0x02]);
        let mut out = Vec::new();
        int(&mut out, 1, -1);
        // Key 0x08, then -1 as ten bytes of two's complement.
        assert_eq!(out.len(), 11);
        assert_eq!((out[0], out[10]), (0x08, 0x01));
    }

    #[test]
    fn a_model_lists_its_nodes_inputs_and_opsets() {
        let mut g = Graph::new();
        let x = g.input("x", DataType::F32, &[Dim::Named("n"), Dim::Fixed(4)]);
        let two = g.scalar(2.0);
        let y = g.node("Mul", &[&x, &two], &[]);
        g.output(&y, "y", DataType::F32, &[Dim::Named("n"), Dim::Fixed(4)]);
        let model = g.model("test", 18);
        let has = |needle: &[u8]| model.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"Mul") && has(b"Identity") && has(b"slopshop"));
        assert!(!has(b"com.microsoft"));
        // ir_version 8 first.
        assert_eq!(&model[..2], &[0x08, 0x08]);
    }

    #[test]
    fn a_contrib_operator_imports_its_domain() {
        let mut g = Graph::new();
        let x = g.input("x", DataType::F16, &[Dim::Fixed(1), Dim::Fixed(32)]);
        let w = g.constant(DataType::U8, &[2, 1, 32], &[128; 64]);
        let y = g.node_microsoft("MatMulNBits", &[&x, &w], &[Attr::Int("bits", 8)]);
        g.output(&y, "y", DataType::F16, &[Dim::Fixed(1), Dim::Fixed(2)]);
        let model = g.model("test", 18);
        let has = |needle: &[u8]| model.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"MatMulNBits") && has(b"com.microsoft"));
    }

    #[test]
    fn external_initializers_name_their_buffer() {
        let mut g = Graph::new();
        g.external("w", DataType::F16, &[2, 3], "weights", 128, 12);
        let model = g.model("test", 18);
        let has = |needle: &[u8]| model.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"location") && has(b"weights") && has(b"128") && has(b"12"));
    }
}
