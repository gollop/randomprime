//! This program traces the dependencies of each pickup in a Metroid Prime ISO.
//! The location of the ISO should be provided as a command line argument.
//!
//! The output has been tailored to match the observed behavior of Miles'
//! randomizer.
//! A few sections of code are commented out, indicating what appear to me to
//! be dependencies, but don't seem to match Miles' dependency lists.

extern crate memmap;
extern crate flate2;
extern crate configurer;

pub use configurer::*;
use configurer::pickup_meta::{/*PickupLocation,*/ ScriptObjectLocation};

use reader_writer::{FourCC, Reader, Writable};
use structs::{Ancs, Cmdl, Evnt, Pickup, Scan, Resource, ResourceKind};

use flate2::{Decompress, Flush};

use std::mem;
use std::env::args;
use std::fs::File;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

// Duplicated from pickup_meta. This version needs owned-lists instead of borrowed.
#[derive(Clone, Debug)]
pub struct PickupLocation
{
    pub location: ScriptObjectLocation,
    pub hudmemo: Option<ScriptObjectLocation>,
}

struct ResourceDb<'a>
{
    map: HashMap<ResourceKey, ResourceDbRecord<'a>>,
}

struct ResourceDbRecord<'a>
{
    data: ResourceData<'a>,
    deps: Option<HashSet<ResourceKey>>,
}

impl<'a> ResourceDb<'a>
{
    fn new() -> ResourceDb<'a>
    {
        ResourceDb {
            map: HashMap::new(),
        }
    }

    fn add_resource(&mut self, res: Resource<'a>)
    {
        let key = ResourceKey::new(res.file_id, res.fourcc());
        self.map.entry(key).or_insert_with(move || {
            let data = ResourceData {
                is_compressed: res.compressed,
                data: match res.kind {
                    ResourceKind::Unknown(reader, _) => reader,
                    _ => panic!("Only uninitialized (aka Unknown) resources may be added."),
                },
            };
            ResourceDbRecord {
                data: data, 
                deps: None,
            }
        });
    }

    fn get_dependencies(&mut self, pickup: &Pickup) -> HashSet<ResourceKey>
    {
        let base_resources = [
            (ResourceKey::new(pickup.cmdl, b"CMDL".into()), None),
            (ResourceKey::new(pickup.ancs.file_id, b"ANCS".into()), Some(pickup.ancs.node_index)),
            (ResourceKey::new(pickup.actor_params.scan_params.scan, b"SCAN".into()), None),
            (ResourceKey::new(pickup.actor_params.xray_cmdl, b"CMDL".into()), None),
            (ResourceKey::new(pickup.actor_params.xray_cskr, b"CSKR".into()), None),
            (ResourceKey::new(pickup.part, b"PART".into()), None),
        ];
        let mut result = HashSet::new();
        for r in base_resources.iter() {
            self.extend_set_with_deps(&mut result, r.0, r.1);
        };
        result
    }

    // XXX We're assuming there are no cycles
    fn get_resource_deps(&mut self, key: ResourceKey, ancs_node: Option<u32>) -> HashSet<ResourceKey>
    {
        let mut deps = HashSet::with_capacity(0);

        let data = {
            let ref record = self.map[&key];
            if let Some(ref deps) = record.deps {
                return deps.clone();
            };
            record.data.clone()
        };
        {
            // To avoid line-wrapping, create a "specialized" version of the method.
            let mut extend_deps = |id, b: &[u8; 4]| {
                self.extend_set_with_deps(&mut deps, ResourceKey::new(id, b.into()), None);
            };

            if key.fourcc == b"SCAN".into() {
                let scan: Scan = data.data.clone().read(());
                extend_deps(scan.frme, b"FRME");
                extend_deps(scan.strg, b"STRG");
            } else if key.fourcc == b"EVNT".into() {
                let evnt: Evnt = data.data.clone().read(());
                for effect in evnt.effect_events.iter() {
                    extend_deps(effect.effect_file_id, effect.effect_type.as_bytes());
                }
            } else if key.fourcc == b"PART".into() {
                let buf = data.decompress();
                let buf: &[u8] = &buf;
                // We're cheating here. We're going to find the sub-string ICTSCNST
                // and then using the next word as the id of a PART.
                //const TOKENS: [&'static [u8]; 3] = [b"ICTSCNST", b"IITSCNST", b"IDTSCNST"];
                for i in 0..(buf.len() - 8) {
                    if &buf[i..(i + 8)] == b"ICTSCNST" {
                        let id : u32 = Reader::new(&buf[(i + 8)..(i+12)]).read(());
                        if id != 0 {
                            extend_deps(id, b"PART");
                        }
                        // TODO: IITS and IDTS too?
                    } else if &buf[i..(i + 4)] == b"TEXR" {
                        if &buf[(i + 4)..(i + 8)] == b"ATEX" {
                            let id : u32 = Reader::new(&buf[(i + 12)..(i + 16)]).read(());
                            if id != 0 {
                                extend_deps(id, b"TXTR");
                            }
                        }
                    } else if &buf[i..(i + 4)] == b"KSSM" && &buf[(i + 4)..(i + 8)] != b"NONE" {

                        let kssm : structs::Kssm = Reader::new(&buf[(i + 8)..]).read(());
                        for list in kssm.lists.iter() {
                            for item in list.items.iter() {
                                extend_deps(item.part, b"PART");
                            }
                        }
                    }
                }
            } else if key.fourcc == b"CMDL".into() {
                let buf = data.decompress();
                let cmdl: Cmdl = Reader::new(&buf).read(());
                for material in cmdl.material_sets.iter() {
                    for id in material.texture_ids.iter() {
                        extend_deps(id, b"TXTR");
                    }
                }
            } else if key.fourcc == b"ANCS".into() {
                let buf = data.decompress();
                let ancs: Ancs = Reader::new(&buf).read(());
                if let Some(ancs_node) = ancs_node {
                    let char_info = ancs.char_set.char_info.iter().nth(ancs_node as usize).unwrap();
                    extend_deps(char_info.cmdl, b"CMDL");
                    extend_deps(char_info.cskr, b"CSKR");
                    extend_deps(char_info.cinf, b"CINF");
                    /*char_info.effects.map(|effects| for effect in effects.iter() {
                        for comp in effect.components.iter() {
                            extend_deps(ResourceKey::new(comp.file_id, comp.type_));
                        }
                    });*/
                    //char_info.overlay_cmdl.map(|cmdl| extend_deps(cmdl, b"CMDL"));
                    //char_info.overlay_cskr.map(|cmdl| extend_deps(cmdl, b"CSKR"));
                    for part in char_info.particles.part_assets.iter() {
                        extend_deps(part, b"PART");
                    }
                };
                ancs.anim_set.animation_resources.map(|i| for anim_resource in i.iter() {
                    extend_deps(anim_resource.anim, b"ANIM");
                    extend_deps(anim_resource.evnt, b"EVNT");
                });
            }
        }

        // We can't safely cache the result if we are using a specific ANCS node.
        // XXX This would be fine if the data structure implementing the cache was
        //     reworked.
        if ancs_node.is_none() {
            self.map.get_mut(&key).unwrap().deps = Some(deps.clone());
        }
        deps
    }

    fn extend_set_with_deps(&mut self, set: &mut HashSet<ResourceKey>, key: ResourceKey,
                                       ancs_node: Option<u32>)
    {
        if key.file_id != u32::max_value() {
            set.insert(key);
            set.extend(self.get_resource_deps(key, ancs_node));
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ResourceKey
{
    file_id: u32, 
    fourcc: FourCC
}

impl ResourceKey
{
    fn new(file_id: u32, fourcc: FourCC) -> ResourceKey
    {
        ResourceKey {
            file_id: file_id,
            fourcc: fourcc,
        }
    }
}

#[derive(Clone, Debug)]
struct ResourceData<'a>
{
    is_compressed: bool,
    data: Reader<'a>,
}


impl<'a> ResourceData<'a>
{
    fn decompress(&self) -> Cow<'a, [u8]>
    {
        if self.is_compressed {
            let mut reader = self.data.clone();
            let size: u32 = reader.read(());
            let _header: u16 = reader.read(());
            // TODO: We could use Vec::set_len to avoid initializing the whole array.
            let mut output = vec![0; size as usize];
            Decompress::new(false).decompress(&reader, &mut output, Flush::Finish).unwrap();

            Cow::Owned(output)
        } else {
            Cow::Borrowed(&self.data)
        }
    }
}


// A map from pickup type -> pickup position
const PICKUP_TYPES: &'static [(usize, &'static str)] = &[
    (1, "Missile"),
    (9, "Energy Tank"),

    (50, "Thermal Visor"),
    (71, "X-Ray Visor"),

    (20, "Varia Suit"),
    (54, "Gravity Suit"),
    (83, "Phazon Suit"),

    (5,  "Morph Ball"),
    (43, "Boost Ball"),
    (44, "Spider Ball"),

    (28, "Morph Ball Bomb"),
    (12, "Power Bomb (small)"),
    (85, "Power Bomb (large)"),

    (23, "Charge Beam"),
    (59, "Space Jump Boots"),
    (75, "Grapple Beam"),

    (47, "Super Missile"),
    (13, "Wavebuster"),
    (96, "Ice Spreader"),
    (76, "Flamethrower"),

    (41, "Wave Beam"),
    (34, "Ice Beam"),
    (99, "Plasma Beam"),

    (14, "Artifact of Lifegiver"),
    (21, "Artifact of Wild"),
    (33, "Artifact of World"),
    (37, "Artifact of Sun"),
    (49, "Artifact of Elder"),
    (56, "Artifact of Spirit"),
    (63, "Artifact of Truth"),
    (73, "Artifact of Chozo"),
    (77, "Artifact of Warrior"),
    (89, "Artifact of Newborn"),
    (91, "Artifact of Nature"),
    (95, "Artifact of Strength"),
];

#[derive(Debug)]
struct PickupData
{
    name: &'static str,
    bytes: Vec<u8>,
    deps: HashSet<ResourceKey>,
    hudmemo_strg: u32,
}

#[derive(Debug)]
struct RoomInfo
{
    room_id: u32,
    pickups: Vec<PickupLocation>,
    objects_to_remove: HashMap<u32, Vec<u32>>,
}


fn trace_pickup_deps(
    gc_disc: &mut structs::GcDisc, pak_name: &str, counter: &mut usize,
    pickup_table: &mut HashMap<usize, PickupData>,
    locations: &mut Vec<Vec<RoomInfo>>,
    cmdl_aabbs: &mut HashMap<u32, [f32; 6]>,
)
{
    let file_entry = find_file(gc_disc, pak_name);
    let pak = match *file_entry.file().unwrap() {
        structs::FstEntryFile::Pak(ref pak) => pak.clone(),
        structs::FstEntryFile::Unknown(ref reader) => reader.clone().read(()),
        _ => panic!(),
    };

    let resources = &pak.resources;

    let mut res_db = ResourceDb::new();
    for res in resources.iter() {
        res_db.add_resource(res.clone());
    }


    locations.push(vec![]);
    let mut locations = locations.last_mut().unwrap();

    for res in resources.iter() {
        if res.fourcc() != b"MREA".into() {
            continue;
        };

        let mut res = res.clone();
        let mrea = res.kind.as_mrea_mut().unwrap();

        let scly = mrea.sections.iter()
            .nth(mrea.scly_section_idx as usize)
            .unwrap().clone();
        let scly: structs::Scly = match scly {
            structs::MreaSection::Unknown(ref reader) => reader.clone().read(()),
            structs::MreaSection::Scly(ref scly) => scly.clone(),
        };

        let mut pickups = vec![];
        let mut scly_db = HashMap::new();
        for (layer_num, scly_layer) in scly.layers.iter().enumerate() {
            for obj in scly_layer.objects.iter() {
                if obj.property_data.object_type() == 0x11 {
                    let mut obj = obj.clone();
                    obj.property_data.guess_kind();
                    let pickup = obj.property_data.as_pickup().unwrap();

                    // We're only interested in "real" pickups
                    if pickup.max_increase > 0 {
                        pickups.push((layer_num, obj.clone()));

                        if pickup.cmdl != u32::max_value() {
                            // Add an aabb entry for this pickup's cmdl
                            cmdl_aabbs.entry(pickup.cmdl).or_insert_with(|| {
                                let cmdl_key = ResourceKey::new(pickup.cmdl, b"CMDL".into());
                                // Cmdls are compressed
                                let res_data = res_db.map[&cmdl_key].data.decompress();
                                let cmdl: Cmdl = Reader::new(&res_data).read(());
                                let aabb = cmdl.maab;
                                // Convert from GenericArray to [f32; 6]
                                [aabb[0], aabb[1], aabb[2], aabb[3], aabb[4], aabb[5]]
                            });
                        }
                    }
                }
                // One of the assets for each pickup is an STRG that is not part of the
                // pickup itself, but is displayed when its acquired. To facilitate finding
                // it, we build a map of all of the scripting objects.
                // XXX The assert checks for SCLY objects with duplicated ids
                assert!(scly_db.insert(obj.instance_id, (layer_num, obj.clone())).is_none());
            }
        }

        for (layer_num, mut obj) in pickups {
            let pickup = obj.property_data.as_pickup_mut().unwrap();
            let mut deps = res_db.get_dependencies(&pickup);

            let mut hudmemo = search_for_scly_object(&obj.connections, &scly_db,
                |obj| obj.property_data.as_hud_memo()
                    .map(|hm| hm.name.to_str().unwrap().contains("Pickup"))
                    .unwrap_or(false)
            );

            let mut removals = Vec::new();
            if pickup.kind >= 29 && pickup.kind <= 40 {
                // If this is an artifact...
                let layer_switch_function = search_for_scly_object(&obj.connections, &scly_db,
                        |obj| obj.property_data.as_special_function()
                            .map(|hm| hm.name.to_str().unwrap()
                                    == "SpecialFunction ScriptLayerController -- Stonehenge Totem")
                            .unwrap_or(false),
                    ).unwrap();
                removals.push(ScriptObjectLocation {
                    layer: scly_db[&layer_switch_function.instance_id].0 as u32,
                    instance_id: layer_switch_function.instance_id,
                });

                let pause_function = search_for_scly_object(&obj.connections, &scly_db,
                        |obj| obj.property_data.as_special_function()
                            .map(|hm| hm.name.to_str().unwrap()
                                    == "SpecialFunction - Enter Logbook Screen")
                            .unwrap_or(false),
                    ).unwrap();
                removals.push(ScriptObjectLocation {
                    layer: scly_db[&pause_function.instance_id].0 as u32,
                    instance_id: pause_function.instance_id,
                });
            }

            let hudmemo_strg;
            if let Some(ref mut hudmemo) = hudmemo {
                let strg = hudmemo.property_data.as_hud_memo().unwrap().strg;
                deps.insert(ResourceKey::new(strg, b"STRG".into()));
                hudmemo_strg = strg;
            } else {
                // Override for the Phazon Suit
                assert_eq!(pickup.kind, 23);
                hudmemo_strg = 0x11BEB861;
                pickup.actor_params.scan_params.scan = 0x50535343;
            }

            patch_dependencies(pickup.kind, &mut deps);

            if let Some(type_id) = PICKUP_TYPES.iter().position(|&(pos, _)| *counter == pos) {
                let mut data = vec![];
                pickup.write(&mut data);
                let name = PICKUP_TYPES[type_id].1;
                pickup_table.insert(type_id, PickupData {
                    name: name,
                    bytes: data,
                    deps: deps,
                    hudmemo_strg: hudmemo_strg
                });
            }

            // TODO: Find a better way to skip this than checking counter
            if *counter != 84 {
                // Skip the extra phazon suit-thing
                let fid = res.file_id;
                let location = PickupLocation {
                    location: ScriptObjectLocation {
                        layer: layer_num as u32,
                        instance_id: obj.instance_id,
                    },
                    hudmemo: hudmemo.map(|obj| ScriptObjectLocation {
                        layer: scly_db[&obj.instance_id].0 as u32,
                        instance_id: obj.instance_id,
                    }),
                };
                if locations.last().map(|i| i.room_id == fid).unwrap_or(false) {
                    locations.last_mut().unwrap().pickups.push(location);
                } else {
                    locations.push(RoomInfo {
                        room_id: res.file_id,
                        pickups: vec![location],
                        objects_to_remove: HashMap::new(),
                    });
                }
                let mut objects_to_remove = &mut locations.last_mut().unwrap().objects_to_remove;
                for r in removals {
                    objects_to_remove.entry(r.layer).or_insert_with(Vec::new).push(r.instance_id);
                }
            }

            *counter += 1;
        }
    }
}

fn search_for_scly_object<'a, F>(
    connections: &reader_writer::LazyArray<'a, structs::Connection>,
    scly_db: &HashMap<u32, (usize, structs::SclyObject<'a>)>,
    f: F
) -> Option<structs::SclyObject<'a>>
    where F: Fn(&structs::SclyObject<'a>) -> bool
{
    let mut stack = Vec::new();

    // Circular references are possible, so keep track of which ones we've seen
    // already.
    let mut seen = HashSet::new();
    
    for c in connections {
        stack.push(c.target_object_id);
        seen.insert(c.target_object_id);
    }

    while let Some(id) = stack.pop() {
        let obj = if let Some(&(_, ref obj)) = scly_db.get(&id) {
            obj
        } else {
            continue;
        };
        if f(&obj) {
            return Some(obj.clone());
        }
        for c in obj.connections.iter() {
            if !seen.contains(&c.target_object_id) {
                stack.push(c.target_object_id);
                seen.insert(c.target_object_id);
            }
        }
    };
    None
}

// We can get pretty close to the Miles' dependecies for each pickup, but some
// of them need custom modification to match exactly.
fn patch_dependencies(pickup_kind: u32, deps: &mut HashSet<ResourceKey>)
{
    // Don't ask me why; Miles seems to skip this one.
    deps.remove(&ResourceKey::new(0xA0DA476B, b"PART".into()));

    if pickup_kind == 19 {
        // Spiderball. I couldn't find any references to this outside of PAK resource
        // indexes and dependency lists.
        deps.insert(ResourceKey::new(0x00656374, b"CSKR".into()));
    } else if pickup_kind == 23 {
        // Phazon suit.
        deps.insert(ResourceKey::new(0x11BEB861, b"STRG".into())); // HudMemo
        deps.insert(ResourceKey::new(0x50535343, b"SCAN".into()));
        deps.insert(ResourceKey::new(0x50535353, b"STRG".into())); // HudMemo
        // TODO: Miles uses a custom texture so it doesn't different from the
        //       gravity suit. Either figure out a replacement or get
        //       permission to use it.
    };
}


fn main()
{
    let file = File::open(args().nth(1).unwrap()).unwrap();
    let mmap = memmap::Mmap::open(&file, memmap::Protection::Read).unwrap();
    let mut reader = Reader::new(unsafe { mmap.as_slice() });
    let mut gc_disc: structs::GcDisc = reader.read(());

    let filenames = [
        "Metroid2.pak",
        "Metroid3.pak",
        "Metroid4.pak",
        "metroid5.pak",
        "Metroid6.pak",
    ];

    let mut i = 0;
    let mut pickup_table = HashMap::new();
    let mut cmdl_aabbs = HashMap::new();
    let mut locations = Vec::new();
    for f in &filenames {
        trace_pickup_deps(&mut gc_disc, f, &mut i, &mut pickup_table, &mut locations,
                          &mut cmdl_aabbs);
    }

    println!("pub const PICKUP_LOCATIONS: [&'static [RoomInfo]; 5] = [");
    for (fname, locations) in filenames.iter().zip(locations.into_iter()) {
        println!("    // {}", fname);
        println!("    &[");
        for room_info in locations {
            println!("        RoomInfo {{");
            println!("            room_id: 0x{:08X},", room_info.room_id);
            println!("            pickup_locations: &[");
            for location in room_info.pickups {
                println!("                PickupLocation {{");
                println!("                    location: {:?},", location.location);
                println!("                    hudmemo: {:?},", location.hudmemo);
                println!("                }},");
            }
            println!("            ],");

            if room_info.objects_to_remove.len() == 0 {
                println!("            objects_to_remove: &[],");
            } else {
                println!("            objects_to_remove: &[");
                let mut objects_to_remove: Vec<_> = room_info.objects_to_remove.iter().collect();
                objects_to_remove.sort_by_key(|&(k, _)| k);
                for otr in objects_to_remove {
                    println!("                ObjectsToRemove {{");
                    println!("                    layer: {},", otr.0);
                    println!("                    instance_ids: &{:?},", otr.1);
                    println!("                }},");
                }
                println!("            ],");
            }
            println!("        }},");
        }
        println!("    ],");
    }
    println!("];");

    println!("const PICKUP_RAW_META: [PickupMetaRaw; 35] = [");
    const BYTES_PER_LINE: usize = 8;
    for i in 0..pickup_table.len() {
        let ref pickup_data = pickup_table[&i];
        let pickup_bytes = &pickup_data.bytes;
        println!("    // {}", pickup_data.name);
        println!("    PickupMetaRaw {{");
        println!("        pickup: &[");
        for y in 0..((pickup_bytes.len() + BYTES_PER_LINE - 1) / BYTES_PER_LINE) {
            let len = ::std::cmp::min(BYTES_PER_LINE, pickup_bytes.len() - y * BYTES_PER_LINE);
            print!("           ");
            for x in 0..len {
                print!(" 0x{:02X},", pickup_bytes[y * BYTES_PER_LINE + x]);
            }
            println!("");
        }
        println!("        ],");
        println!("        deps: &[");
        for dep in &pickup_data.deps {
            println!("            (0x{:08X}, *b\"{}\"),", dep.file_id, dep.fourcc);
        }
        println!("        ],");
        println!("        hudmemo_strg: {:?},", pickup_data.hudmemo_strg);
        println!("    }},");

    }
    println!("];");

    let mut cmdl_aabbs: Vec<_> = cmdl_aabbs.iter().collect();
    cmdl_aabbs.sort_by_key(|&(k, _)| k);
    println!("const PICKUP_CMDL_AABBS: [(u32, [u32; 6]); {}] = [", cmdl_aabbs.len());
    for (cmdl_id, aabb) in cmdl_aabbs {
        let aabb: [u32; 6] = unsafe { mem::transmute(*aabb) };
        println!("    (0x{:08X}, [0x{:08X}, 0x{:08X}, 0x{:08X}, 0x{:08X}, 0x{:08X}, 0x{:08X}]),",
                    cmdl_id, aabb[0], aabb[1], aabb[2], aabb[3], aabb[4], aabb[5]);
    }
    println!("];");
}
