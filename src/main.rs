use std::collections::HashMap;

use stack_graphs::arena::Handle;
use stack_graphs::graph::Node;
use stack_graphs::graph::StackGraph;
use tree_sitter_stack_graphs::loader::LanguageConfiguration;
use tree_sitter_stack_graphs::NoCancellation;
use tree_sitter_stack_graphs_java;

use crate::resolution::ResolutionCtx;
use crate::storage::Store;
use crate::storage::StoreKey;

mod resolution;
mod storage;

static JAVA_FILENAME_1: &'static str = "src/Greeter.java";

static JAVA_SOURCE_1: &'static str = r#"public class Greeter {
    private final String name;

    public Greeter(String name) {
        this.name = name;
    }

    public void sayHello() {
        System.out.println("Hello, " + this.name);
    }
}"#;

static JAVA_FILENAME_2: &'static str = "src/GreeterWrapper.java";

static JAVA_SOURCE_2: &'static str = r#"public class GreeterWrapper {
    private final Greeter greeter;

    public GreeterWrapper(Greeter greeter) {
        this.greeter = greeter;
    }

    public void sayHello() {
        this.greeter.sayHello();
    }
}"#;

fn print_node(graph: &StackGraph, node_handle: Handle<Node>) {
    let node = &graph[node_handle];
    let display = node.display(&graph);

    match graph.source_info(node_handle) {
        None => println!("{}", display),
        Some(info) => {
            let s = info.span.start.as_point();
            let e = info.span.end.as_point();
            let (sr, sc, er, ec) = (s.row, s.column, e.row, e.column);
            println!("{}\t({}, {})\t({}, {})", display, sr, sc, er, ec)
        }
    };
}

fn main() -> anyhow::Result<()> {
    let keys = vec![
        StoreKey::new(String::new(), JAVA_FILENAME_1.to_string()),
        StoreKey::new(String::new(), JAVA_FILENAME_2.to_string()),
    ];

    let mut sources = HashMap::new();
    sources.insert(keys[0].clone(), JAVA_SOURCE_1.to_string());
    sources.insert(keys[1].clone(), JAVA_SOURCE_2.to_string());

    let mut store = Store::open("./neodepends.db")?;

    for key in &store.find_missing(&keys)? {
        println!("Missing {}", key);
        let mut ctx = ResolutionCtx::from_source(&sources[key], &key.filename)?;
        store.save(key, &mut ctx)?;
    }

    let mut ctx = store.load(&keys)?;

    for (src, tgt) in ctx.resolve() {
        println!("{} -> {}", src, tgt);
    }

    // references.sort_by(|a, b| {
    //     let a_0 = &res_ctx.graph[a.start_node()].id();
    //     let b_0 = &res_ctx.graph[b.start_node()].id();
    //     let a_1 = &res_ctx.graph[a.end_node()].id();
    //     let b_1 = &res_ctx.graph[b.end_node()].id();
    //     (a_0, a_1).cmp(&(b_0, b_1))
    // });

    // for reference in references {
    //     print_node(&res_ctx.graph, reference.start_node);
    //     print_node(&res_ctx.graph, reference.end_node);
    //     println!();
    // }

    Ok(())
}
