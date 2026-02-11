#[macro_export]
macro_rules! define_cid_map {
    ($($index: literal -> $ty: expr),* $(,)?) => {{
    let mut map = ::std::collections::HashMap::new();
    let mut elements: std::vec::Vec<::std::boxed::Box<dyn Component>> = ::std::vec::Vec::new();
    $(
        $ty.set_index($index);
        elements.push(::std::boxed::Box::new($ty));
        map.insert($index, elements.len() - 1);
    )*
    $crate::ui::CIDMAP.set(
    map
    ).map_err(|_| $crate::errors::AppError::ErrorSettingGlobal("CTIDMAP"))?;
        Result::<std::vec::Vec<::std::boxed::Box<dyn Component>>, AppError>::Ok(elements)
    }}
}
