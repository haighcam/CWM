use cwm::utils::Stack;

fn main() {
    let mut stack = Stack::default();
    let mut items: Vec<_> = (0..5).map(|x| stack.push_front(x)).collect();
    println!("{:#?}", stack);
    stack.remove_node(items[4]);
    println!("{:#?}", stack);
    items[4] = stack.push_back(4);
    println!("{:#?}", stack);
    stack.remove_node(items[0]);
    items[0] = stack.push_front(0);
    println!("{:?}", stack.iter().collect::<Vec<_>>());
    println!("{:?}", items);
    
}