use super::*;

fn key(code: u16) -> Held {
    Held::Key(Key::new(code).unwrap())
}

#[test]
fn every_key_and_button_has_a_cell_of_its_own_inside_the_table() {
    let table = Table::private();
    let mut items: Vec<Held> = Key::CODES.map(key).collect();
    items.extend(
        [
            Button::Left,
            Button::Right,
            Button::Middle,
            Button::Side,
            Button::Extra,
        ]
        .map(Held::Button),
    );
    for &item in &items {
        table.mark(item);
    }
    let bytes = table.copy();
    assert_eq!(bytes.iter().filter(|&&byte| byte != 0).count(), items.len());
    assert_eq!(Owed::parse(&bytes).items().len(), items.len());
    for &item in &items {
        table.clear(item);
        assert!(!table.owes(item));
    }
}

#[test]
fn a_hostile_table_yields_releases_of_real_keys_and_buttons_only() {
    let mut bytes = [0xffu8; TABLE_LEN];
    // Nothing owed at three places, so they are not in the answer.
    bytes[30] = 0;
    bytes[0x111] = 0;
    bytes[1] = 0;
    let owed = Owed::parse(&bytes);
    let keys = owed
        .items()
        .iter()
        .filter(|item| matches!(item, Held::Key(_)))
        .count();
    let buttons = owed
        .items()
        .iter()
        .filter(|item| matches!(item, Held::Button(_)))
        .count();
    // Keys 2..=248 but 30; buttons left, middle, side and extra.
    assert_eq!((keys, buttons), (246, 4));
    assert!(!owed.items().contains(&key(30)));
    // Code 0, 249..=0x10f, 0x115..=0x2ff: no key and no button.
    assert_eq!(owed.rejected, 1 + (0x10f - 249 + 1) + (0x2ff - 0x115 + 1));
}

#[test]
fn any_value_but_zero_owes_and_an_empty_table_owes_nothing() {
    assert!(Owed::parse(&[0; TABLE_LEN]).items().is_empty());
    let mut bytes = [0u8; TABLE_LEN];
    bytes[17] = 0x80;
    bytes[0x110] = 2;
    assert_eq!(
        Owed::parse(&bytes).items(),
        [key(17), Held::Button(Button::Left)]
    );
}

#[test]
fn the_section_maps_twice_onto_the_same_cells() {
    let section = Section::create().unwrap();
    let second = map(section.raw()).unwrap();
    section.table().mark(key(17));
    assert!(second.owes(key(17)));
    second.clear(key(17));
    assert!(!section.table().owes(key(17)));
}
