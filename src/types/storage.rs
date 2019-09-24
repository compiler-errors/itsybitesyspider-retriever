use super::entry::Entry;
use crate::internal::hasher::HasherImpl;
use crate::internal::mr::mrvec::MrVec;
use crate::traits::query::Query;
use crate::traits::record::Record;
use crate::traits::valid_key::ValidKey;
use crate::types::arc_iter::ArcIter;
use crate::types::editor::Editor;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::chunk_storage::*;

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Chunked, indexed storage
#[derive(Clone)]
pub struct Storage<ChunkKey, ItemKey, Element> {
    id: u64,
    chunks: MrVec<ChunkStorage<ChunkKey, ItemKey, Element>>,
    dirty: Vec<usize>,
    index: HashMap<ChunkKey, usize, HasherImpl>,
}

impl<ChunkKey, ItemKey, Element> Storage<ChunkKey, ItemKey, Element>
where
    ChunkKey: ValidKey,
    ItemKey: ValidKey,
    Element: Record<ChunkKey, ItemKey>,
{
    /// Construct a new Storage.
    ///
    /// ```
    /// use retriever::*;
    ///
    /// let mut storage : Storage<u64, &'static str, _> = Storage::new();
    ///
    /// // In a later example, we'll encourage jroberts to use a stronger password.
    /// let user_id = 7;
    /// let username = String::from("jroberts");
    /// let password = String::from("PASSWORD!5");
    /// let admin = String::from("true");
    ///
    /// // For this example we choose a storage that represents some account information for a
    /// // single user. The tuple (Key, Value) type has a built-in impl for Record.
    /// storage.add((user_id, "username", username.clone()));
    /// storage.add((user_id, "password", password.clone()));
    /// storage.add((user_id, "admin", admin.clone()));
    ///
    /// // We can lookup the value of the "admin" field using it's item key.
    /// let is_admin = storage.get(&ID.chunk(user_id).item("admin"));
    /// assert_eq!(is_admin, Some(&(7, "admin",admin.clone())));
    ///
    /// # storage.validate();
    /// ```
    pub fn new() -> Self {
        Storage {
            id: ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            chunks: MrVec::default(),
            dirty: Vec::default(),
            index: HashMap::with_hasher(crate::internal::hasher::HasherImpl::default()),
        }
    }

    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// Get the ChunkStorage corresponding the given ChunkKey.
    fn chunk(
        &mut self,
        chunk_key: &ChunkKey,
        dirty: bool,
    ) -> &mut ChunkStorage<ChunkKey, ItemKey, Element> {
        let idx = if let Some(idx) = self.internal_idx_of(chunk_key) {
            idx
        } else {
            let new_idx = self.chunks.len();
            self.index.insert(chunk_key.clone(), new_idx);
            self.chunks.push(ChunkStorage::new(chunk_key.clone()));
            new_idx
        };

        if dirty {
            self.dirty(idx);
        }

        &mut self.chunks[idx]
    }

    /// Add the given element to this Storage.
    ///
    /// ```
    /// use retriever::*;
    /// use std::borrow::Cow;
    ///
    /// // This example will be a database of student records.
    /// struct Student {
    ///   school: String,
    ///   id: u64,
    ///   first_name: String,
    ///   last_name: String,
    /// }
    ///
    /// // Do note! Using the school name as the chunk key does mean that we'll have to
    /// // delete and re-add students who move to a different school.
    /// impl Record<String, u64> for Student {
    ///   fn chunk_key(&self) -> Cow<String> {
    ///     Cow::Borrowed(&self.school)
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<u64> {
    ///     Cow::Owned(self.id)
    ///   }
    /// }
    ///
    /// let mut storage : Storage<String, u64, Student> = Storage::new();
    ///
    /// storage.add(Student {
    ///   school: String::from("PS109"),
    ///   id: 89875,
    ///   first_name: String::from("Mary"),
    ///   last_name: String::from("Jones"),
    /// });
    ///
    /// storage.add(Student {
    ///   school: String::from("PS109"),
    ///   id: 99200,
    ///   first_name: String::from("Alisha"),
    ///   last_name: String::from("Wu"),
    /// });
    ///
    /// storage.add(Student {
    ///   school: String::from("Northwood Elementary"),
    ///   id: 01029,
    ///   first_name: String::from("Anders"),
    ///   last_name: String::from("McAllister"),
    /// });
    ///
    /// let anders = storage.get(&ID.chunk(String::from("Northwood Elementary")).item(01029));
    /// assert_eq!(&anders.unwrap().first_name, "Anders");
    ///
    /// # storage.validate();
    /// ```
    pub fn add(&mut self, element: Element) -> &mut Self {
        self.clean();

        let chunk_key = element.chunk_key();
        let chunk_key_ref = chunk_key.borrow();
        self.chunk(chunk_key_ref, false).add(element);

        self
    }

    fn clean(&mut self) {
        if self.dirty.is_empty() {
            return;
        }

        self.dirty.sort_unstable();

        for idx in self.dirty.iter().rev() {
            if self.chunks[*idx].len() > 0 {
                continue;
            }

            self.index.remove(self.chunks[*idx].chunk_key());
            self.chunks.swap_remove(*idx);
        }

        self.dirty.clear();
    }

    fn dirty(&mut self, idx: usize) {
        self.dirty.push(idx);
    }

    /// Dissolve this Storage into a list of chunks.
    pub fn dissolve(self) -> impl IntoIterator<Item = Vec<Element>> {
        let chunks: Vec<_> = self.chunks.into();
        chunks.into_iter().map(|chunk| chunk.into())
    }

    /// Raw serial access to all element data by reference.
    /// (Tip! You can use Serde to serialize a list of element references and later deserialize them as values.)
    ///
    /// ```
    /// use retriever::*;
    ///
    /// // Load some data into storage.
    /// let mut storage : Storage<(), usize, (usize, &'static str)> = Storage::new();
    ///
    /// storage.add((0, "hello"));
    /// storage.add((1, "doctor"));
    /// storage.add((2, "name"));
    /// storage.add((3, "continue"));
    /// storage.add((4, "yesterday"));
    /// storage.add((5, "tomorrow"));
    ///
    /// // Now create a second storage mirroring the data of the first by-reference.
    /// // References to Records are also Records with the same key types.
    /// let mut mirror : Storage<(), usize, &(usize, &'static str)> = Storage::new();
    ///
    /// for chunk in storage.raw().into_iter() {
    ///   for item_reference in chunk {
    ///     mirror.add(item_reference);
    ///   }
    /// }
    ///
    /// // Notice the double reference, because we are getting a reference to an element in 'mirror'
    /// // that is itself a reference to an owned element in 'storage'.
    /// let yesterday = mirror.get(&ID.item(4));
    /// assert_eq!(yesterday, Some(&&(4, "yesterday")));
    ///
    /// # storage.validate();
    /// ```
    pub fn raw(&self) -> impl IntoIterator<Item = &[Element]> {
        self.chunks.iter().map(|chunk| chunk.raw())
    }

    /// Get a data element. A data element is uniquely identified by the combination of it's
    /// chunk key and item key. Accordingly, you can look up any Record using another Record
    /// that supports the same key types.
    ///
    /// Returns None if the data element does not exist.
    ///
    /// ```
    /// use retriever::*;
    /// use std::borrow::Cow;
    ///
    /// #[derive(Clone)]
    /// struct RetailStore {
    ///   region: String,
    ///   number: usize,
    /// }
    ///
    /// struct Accounting {
    ///   store: RetailStore,
    ///   revenue: u64,
    ///   expenses: u64,
    ///   taxes: u64,
    ///   fines: u64,
    /// }
    ///
    /// struct Marketing {
    ///   store: RetailStore,
    ///   slogan: String,
    /// }
    ///
    /// impl Record<String, usize> for RetailStore {
    ///   fn chunk_key(&self) -> Cow<String> {
    ///     Cow::Borrowed(&self.region)
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<usize> {
    ///     Cow::Owned(self.number)
    ///   }
    /// }
    ///
    /// impl Record<String, usize> for Accounting {
    ///   fn chunk_key(&self) -> Cow<String> {
    ///     self.store.chunk_key()
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<usize> {
    ///     self.store.item_key()
    ///   }
    /// }
    ///
    /// impl Record<String, usize> for Marketing {
    ///   fn chunk_key(&self) -> Cow<String> {
    ///     self.store.chunk_key()
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<usize> {
    ///     self.store.item_key()
    ///   }
    /// }
    ///
    /// let mut accounting : Storage<String, usize, Accounting> = Storage::new();
    /// let mut marketing : Storage<String, usize, Marketing> = Storage::new();
    ///
    /// let store = RetailStore { region: String::from("North"), number: 7 };
    ///
    /// accounting.add(Accounting {
    ///   store: store.clone(),
    ///   revenue: 1300000,
    ///   expenses: 1100000,
    ///   taxes: 100,
    ///   fines: 100000,
    /// });
    ///
    /// marketing.add(Marketing {
    ///   store: store.clone(),
    ///   slogan: String::from("You want to buy from us, today!"),
    /// });
    ///
    /// // Lookup using the fluent constructor syntax starting with the empty Id called ID.
    /// assert_eq!(
    ///   accounting.get(&ID.chunk(String::from("North")).item(7)).map(|x| x.revenue),
    ///   Some(1300000),
    /// );
    ///
    /// // Lookup using tuple record syntax. We can always look up any record
    /// // using any other record with the same key types.
    /// assert_eq!(
    ///   accounting.get(&(String::from("North"),7,())).map(|x| x.revenue),
    ///   Some(1300000),
    /// );
    ///
    /// // Lookup using RetailStore record.
    /// assert_eq!(
    ///   accounting.get(&store).map(|x| x.revenue),
    ///   Some(1300000),
    /// );
    ///
    /// // Lookup the slogans for all profitable stores. This performs a lookup of a marketing
    /// // record using the corresponding accounting record as the key.
    /// let mut count = 0;
    /// for store in accounting.iter() {
    ///   let profit = store.revenue - store.expenses - store.taxes - store.fines;
    ///
    ///   if let Some(slogan) = marketing.get(&store).map(|x| &x.slogan) {
    ///     count += 1;
    ///     println!("{}: Store {}-{} (profit: {}) has the slogan: {}",
    ///       count,
    ///       &store.store.region,
    ///       store.store.number,
    ///       profit,
    ///       slogan);
    ///   }
    /// }
    ///
    /// assert_eq!(count, 1);
    ///
    /// # accounting.validate();
    /// # marketing.validate();
    /// ```
    pub fn get<R>(&self, unique_id: &R) -> Option<&Element>
    where
        R: Record<ChunkKey, ItemKey>,
    {
        self.internal_idx_of(unique_id.borrow().chunk_key().borrow())
            .and_then(|idx| self.chunks[idx].get(unique_id))
    }

    /// Get an Entry for a data element, which supports mutation.
    ///
    /// The Entry API is very similar, but not identical to, the Entry APIs supported by rust's
    /// standard collections framework.
    ///
    /// Since re-indexing is a potentially expensive operation, it's best to examine an immutable
    /// reference to a data element to make sure you really want to mutate it before obtaining a
    /// mutable reference.
    ///
    /// ```
    /// use retriever::*;
    ///
    /// let mut storage : Storage<(),usize,(usize,f64)> = Storage::new();
    ///
    /// storage.entry(&(0,())).or_insert_with(|| (0,4.0));
    /// assert_eq!(storage.get(&ID.item(0)), Some(&(0,4.0)));
    ///
    /// storage.entry(&(0,())).or_insert_with(|| (0,9.0)).and_modify(|(_,x)| {
    ///   *x = x.sqrt();
    /// });
    /// assert_eq!(storage.get(&ID.item(0)), Some(&(0,2.0)));
    ///
    /// storage.entry(&(0,())).remove();
    /// assert_eq!(storage.get(&ID.item(0)), None);
    ///
    /// # storage.validate();
    /// ```
    pub fn entry<'a, R>(&'a mut self, unique_id: &'a R) -> Entry<'a, ChunkKey, ItemKey, Element>
    where
        R: Record<ChunkKey, ItemKey> + 'a,
    {
        self.clean();
        self.chunk(unique_id.borrow().chunk_key().borrow(), true)
            .entry(unique_id)
    }

    /// Iterate over all elements in this storage.
    pub fn iter(&self) -> impl Iterator<Item = &Element> {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    /// Iterate over all elements of a shared reference to this storage.
    pub fn iter_arc(arc: Arc<Self>) -> ArcIter<ChunkKey, ItemKey, Element>
    where
        Element: Clone,
    {
        ArcIter::new(arc)
    }

    /// Iterate over elements according to some Query. A variety of builtin queries are provided.
    /// The simplest useful query is Everything, which iterates over everything.
    ///
    /// ```
    /// use retriever::*;
    ///
    /// let mut storage : Storage<usize,usize,(usize,usize,i64)> = Storage::new();
    ///
    /// storage.add((0,0,17));
    /// storage.add((0,1,53));
    /// storage.add((0,9,-57));
    /// storage.add((1,7,29));
    /// storage.add((2,13,-19));
    /// storage.add((2,1024,-23));
    ///
    /// // All elements together should sum to zero:
    /// assert_eq!(storage.iter().map(|(_,_,x)| *x).sum::<i64>(), 0);
    ///
    /// # storage.validate();
    /// ```
    pub fn query<'a, Q>(&'a self, query: &'a Q) -> impl Iterator<Item = &'a Element>
    where
        Q: Query<ChunkKey, ItemKey, Element>,
    {
        let chunk_idxs = query.chunk_idxs(&self);

        chunk_idxs
            .iter()
            .map(move |idx| &self.chunks[idx])
            .flat_map(move |chunk_storage| chunk_storage.query(query))
    }

    /// Iterate over a Query and modify each element via a callback.
    /// The callback provides retriever's Editor API, which in turn provides
    /// a mutable or immutable reference to the underlying element.
    ///
    /// Since re-indexing is a potentially expensive operation, it's best to examine an immutable
    /// reference to a data element to make sure you really want to mutate it before obtaining a
    /// mutable reference.
    ///
    /// ```
    /// use retriever::*;
    /// use retriever::queries::everything::Everything;
    /// use std::borrow::Cow;
    ///
    /// struct BankAccount {
    ///   id: usize,
    ///   balance: i64,
    /// }
    ///
    /// impl Record<(),usize> for BankAccount {
    ///   fn chunk_key(&self) -> Cow<()> {
    ///     Cow::Owned(())
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<usize> {
    ///     Cow::Owned(self.id)
    ///   }
    /// }
    ///
    /// let mut storage : Storage<(),usize,BankAccount> = Storage::new();
    ///
    /// storage.add(BankAccount { id: 1, balance: 25 });
    /// storage.add(BankAccount { id: 2, balance: 13 });
    /// storage.add(BankAccount { id: 3, balance: 900 });
    /// storage.add(BankAccount { id: 4, balance: 27000 });
    /// storage.add(BankAccount { id: 5, balance: -13 });
    ///
    /// // Charge an overdraft fee to everyone with a negative balance.
    /// // Take care to check that the account balance is actually negative
    /// // before asking for a mutable reference, because just getting the
    /// // mutable reference might trigger an expensive re-indexing operation.
    /// storage.modify(Everything, |mut account| {
    ///   if account.get().balance < 0 {
    ///     account.get_mut().balance -= 25;
    ///   }
    /// });
    ///
    /// assert_eq!(27900,storage.iter().map(|account| account.balance).sum::<i64>());
    ///
    /// # storage.validate();
    /// ```
    pub fn modify<Q, F>(&mut self, mut query: Q, f: F)
    where
        Q: for<'x, 'y> Query<ChunkKey, ItemKey, Element>,
        F: Fn(Editor<ChunkKey, ItemKey, Element>),
    {
        self.clean();

        for idx in query.chunk_idxs(self).iter() {
            self.chunks[idx].modify(&mut query, &f);
        }
    }

    /// Discard all of the specified elements from this storage.
    ///
    /// ```
    /// use retriever::*;
    /// use retriever::queries::everything::Everything;
    /// use std::borrow::Cow;
    ///
    /// // In this example, we will store log entries, some of which might contain sensitive
    /// // information that we need to delete later.
    /// struct LogEntry {
    ///   stardate: u128,
    ///   msg: String,
    /// }
    ///
    /// impl Record<u128, u128> for LogEntry {
    ///   fn chunk_key(&self) -> Cow<u128> {
    ///     Cow::Owned(self.stardate / 1000)
    ///   }
    ///
    ///   fn item_key(&self) -> Cow<u128> {
    ///     Cow::Borrowed(&self.stardate)
    ///   }
    /// }
    ///
    /// let mut storage : Storage<u128, u128, LogEntry> = Storage::new();
    ///
    /// storage.add(LogEntry {
    ///   stardate: 109301,
    ///   msg: String::from("Departed from Starbase Alpha"),
    /// });
    ///
    /// storage.add(LogEntry {
    ///   stardate: 109302,
    ///   msg: String::from("Purchased illegal cloaking device from aliens"),
    /// });
    ///
    /// storage.add(LogEntry {
    ///   stardate: 109303,
    ///   msg: String::from("Asked doctor to check cat for space fleas"),
    /// });
    ///
    /// // Use the 'discard' operation to search for any embarassing log entries
    /// // and discard them.
    /// storage.discard(Everything.filter(|log_entry: &LogEntry| {
    ///   log_entry.msg.contains("illegal")
    /// }));
    ///
    /// assert!(
    ///   storage
    ///     .get(&ID.chunk(109).item(109302))
    ///     .is_none());
    ///
    /// assert_eq!(
    ///   storage.iter().count(),
    ///   2);
    ///
    /// # storage.validate();
    /// ```
    pub fn discard<Q>(&mut self, mut query: Q)
    where
        Q: Query<ChunkKey, ItemKey, Element>,
    {
        for idx in query.chunk_idxs(self).iter() {
            self.dirty(idx);
            self.chunks[idx].discard(&mut query);
        }

        self.clean();
    }

    /// Remove and return all of the specified elements from this storage.
    pub fn remove<Q>(&mut self, mut query: Q) -> Vec<Vec<Element>>
    where
        Q: Query<ChunkKey, ItemKey, Element>,
    {
        let mut result = Vec::new();

        for idx in query.chunk_idxs(self).iter() {
            self.dirty(idx);
            result.push(self.chunks[idx].remove(&mut query));
        }

        self.clean();

        result
    }

    /// List all chunks
    pub fn chunk_keys(&self) -> impl IntoIterator<Item = &ChunkKey> {
        self.chunks.iter().map(|chunk| chunk.chunk_key())
    }

    /// Drop an entire chunk and return all associated elements
    pub fn remove_chunk(&mut self, chunk_key: &ChunkKey) -> Option<Vec<Element>> {
        self.clean();
        let idx = self.index.remove(chunk_key)?;
        let chunk = self.chunks.swap_remove(idx);
        Some(chunk.into())
    }

    /// Panic if this storage is malformed or broken in any way.
    pub fn validate(&mut self) {
        self.clean();

        for (idx, chunk) in self.chunks.iter().enumerate() {
            assert_eq!(
                self.index.get(chunk.chunk_key()),
                Some(&idx),
                "chunk not indexed"
            );
        }

        for (chunk_key, idx) in self.index.iter() {
            assert_eq!(self.chunks[*idx].chunk_key(), chunk_key, "index broken");
            assert_ne!(self.chunks[*idx].len(), 0, "empty chunk");
        }

        for chunk in self.chunks.iter() {
            chunk.validate();
        }
    }

    pub(crate) fn internal_idx_of<Q>(&self, chunk_key: &Q) -> Option<usize>
    where
        Q: Hash + Eq,
        ChunkKey: Borrow<Q>,
    {
        self.index.get(chunk_key).cloned()
    }

    pub(crate) fn internal_mrvec(&self) -> &MrVec<ChunkStorage<ChunkKey, ItemKey, Element>> {
        &self.chunks
    }

    /// Given a list of ChunkKeys updated by previous calls to gc(), delete all the ChunkKeys
    /// that no longer exist in the specified HashMap.
    pub(crate) fn gc<T>(
        &self,
        chunk_list: &mut MrVec<Option<ChunkKey>>,
        data: &mut HashMap<ChunkKey, T, crate::internal::hasher::HasherImpl>,
    ) {
        let mut removed: HashSet<ChunkKey, _> =
            HashSet::with_hasher(crate::internal::hasher::HasherImpl::default());
        let mut added: HashSet<ChunkKey, _> =
            HashSet::with_hasher(crate::internal::hasher::HasherImpl::default());

        chunk_list.map_reduce(&self.chunks, 1, |chunk_storages, prev_chunk_key, _| {
            if chunk_storages.is_empty() {
                if let Some(chunk_key) = prev_chunk_key.as_ref() {
                    removed.insert(chunk_key.clone());
                }
                None
            } else if Some(chunk_storages[0].chunk_key()) != prev_chunk_key.as_ref() {
                added.insert(chunk_storages[0].chunk_key().clone());
                if let Some(chunk_key) = prev_chunk_key.as_ref() {
                    removed.insert(chunk_key.clone());
                }
                Some(Some(chunk_storages[0].chunk_key().clone()))
            } else {
                None
            }
        });

        for chunk_key in removed.difference(&added) {
            data.remove(chunk_key);
        }
    }
}

impl<ChunkKey, ItemKey, Element> Default for Storage<ChunkKey, ItemKey, Element>
where
    ChunkKey: ValidKey,
    ItemKey: ValidKey,
    Element: Record<ChunkKey, ItemKey>,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use crate::queries::everything::Everything;
    use crate::queries::secondary_index::SecondaryIndex;
    use crate::*;
    use std::borrow::Cow;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct X(u64, u64);

    impl Record<u64, u64> for X {
        fn chunk_key(&self) -> Cow<u64> {
            Cow::Owned((self.0 & 0x00F0) >> 4)
        }

        fn item_key(&self) -> Cow<u64> {
            Cow::Borrowed(&self.0)
        }
    }

    #[test]
    fn test_remove_and_replace_chunk_with_secondary_index() {
        let mut storage: Storage<u64, u64, X> = Storage::new();
        let mut index: SecondaryIndex<u64, X, Option<u64>, u64> =
            SecondaryIndex::new_expensive(&storage, |x: &X| Some(x.1 & 0x1));

        storage.add(X(0x101, 0x101));
        storage.add(X(0x102, 0x102));
        storage.add(X(0x103, 0x103));
        storage.add(X(0x104, 0x104));
        storage.add(X(0x105, 0x105));
        storage.add(X(0x106, 0x106));
        storage.add(X(0x107, 0x107));
        storage.add(X(0x108, 0x108));

        assert_eq!(
            4,
            storage.query(&Everything.matching(&mut index, &0)).count()
        );

        storage.remove_chunk(&0);

        storage.add(X(0x101, 0x101));
        storage.add(X(0x102, 0x102));
        storage.add(X(0x103, 0x103));
        storage.add(X(0x104, 0x104));
        storage.add(X(0x105, 0x105));
        storage.add(X(0x106, 0x106));
        storage.add(X(0x107, 0x107));
        storage.add(X(0x108, 0x108));

        assert_eq!(
            4,
            storage.query(&Everything.matching(&mut index, &0)).count()
        );
    }
}