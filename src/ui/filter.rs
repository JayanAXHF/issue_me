pub struct Filter<P, T>
where
    P: for<'a> Fn(&T) -> bool,
{
    filter_fn: P,
    _phantom: std::marker::PhantomData<T>,
}

impl<P, T> Filter<P, T>
where
    P: Fn(&T) -> bool,
{
    pub fn new(filter_fn: P) -> Self {
        Self {
            filter_fn,
            _phantom: Default::default(),
        }
    }
}

pub trait FilterExt<P, T>
where
    P: for<'a> Fn(&T) -> bool,
{
    type Output;
    fn filter(&self, filter: &Filter<P, T>) -> Self::Output;
}

impl<P, T> FilterExt<P, T> for Vec<T>
where
    P: Fn(&T) -> bool,
    T: Clone,
{
    type Output = Vec<T>;
    fn filter(&self, filter: &Filter<P, T>) -> Self::Output {
        self.iter()
            .filter(|x| (filter.filter_fn)(x))
            .cloned()
            .collect()
    }
}
