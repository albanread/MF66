use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Clone,
            Copy,
            Debug,
            Default,
            Eq,
            Hash,
            Ord,
            PartialEq,
            PartialOrd,
            Deserialize,
            Serialize,
        )]
        pub struct $name(pub u64);

        impl $name {
            pub const fn new(value: u64) -> Self {
                Self(value)
            }

            pub const fn get(self) -> u64 {
                self.0
            }

            pub const fn is_zero(self) -> bool {
                self.0 == 0
            }
        }
    };
}

id_type!(WorkerId);
id_type!(SessionId);
id_type!(PaneId);
id_type!(TaskId);
id_type!(BulkId);
id_type!(CorrelationId);
id_type!(Seq);
id_type!(TimerId);
