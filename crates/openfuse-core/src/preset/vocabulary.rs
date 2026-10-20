//! Factual annotation vocabulary recovered from FUSE 2.7.1.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Genre {
    pub id: i32,
    pub label: &'static str,
}

pub const GENRES: &[Genre] = &[
    Genre {
        id: -1,
        label: "None",
    },
    Genre {
        id: 0,
        label: "Acoustic",
    },
    Genre {
        id: 1,
        label: "Alternative",
    },
    Genre {
        id: 2,
        label: "Alternative rock",
    },
    Genre {
        id: 3,
        label: "Americana",
    },
    Genre {
        id: 4,
        label: "Ballad",
    },
    Genre {
        id: 5,
        label: "Bebop",
    },
    Genre {
        id: 6,
        label: "Blues",
    },
    Genre {
        id: 7,
        label: "Classical",
    },
    Genre {
        id: 8,
        label: "Classic Rock",
    },
    Genre {
        id: 9,
        label: "Country",
    },
    Genre {
        id: 10,
        label: "Dance",
    },
    Genre {
        id: 11,
        label: "Death Metal",
    },
    Genre {
        id: 12,
        label: "Disco",
    },
    Genre {
        id: 13,
        label: "Electronic",
    },
    Genre {
        id: 14,
        label: "Folk",
    },
    Genre {
        id: 15,
        label: "Folk Rock",
    },
    Genre {
        id: 16,
        label: "Funk",
    },
    Genre {
        id: 17,
        label: "Fusion",
    },
    Genre {
        id: 18,
        label: "Gospel",
    },
    Genre {
        id: 19,
        label: "Grunge",
    },
    Genre {
        id: 20,
        label: "Hard Rock",
    },
    Genre {
        id: 21,
        label: "Hip Hop",
    },
    Genre {
        id: 22,
        label: "Indie",
    },
    Genre {
        id: 23,
        label: "Industrial",
    },
    Genre {
        id: 24,
        label: "Instrumental",
    },
    Genre {
        id: 25,
        label: "Jazz",
    },
    Genre {
        id: 26,
        label: "Latin",
    },
    Genre {
        id: 27,
        label: "Metal",
    },
    Genre {
        id: 28,
        label: "New Age",
    },
    Genre {
        id: 29,
        label: "New Wave",
    },
    Genre {
        id: 30,
        label: "Oldies",
    },
    Genre {
        id: 31,
        label: "Other",
    },
    Genre {
        id: 32,
        label: "Pop",
    },
    Genre {
        id: 33,
        label: "Progressive Rock",
    },
    Genre {
        id: 34,
        label: "Psychedelic",
    },
    Genre {
        id: 35,
        label: "Punk",
    },
    Genre {
        id: 36,
        label: "Punk Rock",
    },
    Genre {
        id: 37,
        label: "Rap",
    },
    Genre {
        id: 38,
        label: "Reggae",
    },
    Genre {
        id: 39,
        label: "R&B",
    },
    Genre {
        id: 40,
        label: "Rock",
    },
    Genre {
        id: 41,
        label: "Rockabilly",
    },
    Genre {
        id: 42,
        label: "Rock 'n' roll",
    },
    Genre {
        id: 43,
        label: "Ska",
    },
    Genre {
        id: 44,
        label: "Soul",
    },
    Genre {
        id: 45,
        label: "Soundtrack",
    },
    Genre {
        id: 46,
        label: "Southern Rock",
    },
    Genre {
        id: 47,
        label: "Swing",
    },
    Genre {
        id: 48,
        label: "Techno",
    },
    Genre {
        id: 49,
        label: "A cappella",
    },
    Genre {
        id: 50,
        label: "Acid",
    },
    Genre {
        id: 51,
        label: "Acid Jazz",
    },
    Genre {
        id: 52,
        label: "Acid Punk",
    },
    Genre {
        id: 53,
        label: "Ambient",
    },
    Genre {
        id: 54,
        label: "Avantgarde",
    },
    Genre {
        id: 55,
        label: "Bass",
    },
    Genre {
        id: 56,
        label: "Big Band",
    },
    Genre {
        id: 57,
        label: "Bluegrass",
    },
    Genre {
        id: 58,
        label: "Booty Bass",
    },
    Genre {
        id: 59,
        label: "Cabaret",
    },
    Genre {
        id: 60,
        label: "Celtic",
    },
    Genre {
        id: 61,
        label: "Chamber Music",
    },
    Genre {
        id: 62,
        label: "Chanson",
    },
    Genre {
        id: 63,
        label: "Chorus",
    },
    Genre {
        id: 64,
        label: "Christian Rap",
    },
    Genre {
        id: 65,
        label: "Club",
    },
    Genre {
        id: 66,
        label: "Comedy",
    },
    Genre {
        id: 67,
        label: "Cult",
    },
    Genre {
        id: 68,
        label: "Dance Hall",
    },
    Genre {
        id: 69,
        label: "Darkwave",
    },
    Genre {
        id: 70,
        label: "Dream",
    },
    Genre {
        id: 71,
        label: "Drum Solo",
    },
    Genre {
        id: 72,
        label: "Duet",
    },
    Genre {
        id: 73,
        label: "Easy Listening",
    },
    Genre {
        id: 74,
        label: "Ethnic",
    },
    Genre {
        id: 75,
        label: "Eurodance",
    },
    Genre {
        id: 76,
        label: "Euro House",
    },
    Genre {
        id: 77,
        label: "Euro Techno",
    },
    Genre {
        id: 78,
        label: "Fast Fusion",
    },
    Genre {
        id: 79,
        label: "Folklore",
    },
    Genre {
        id: 80,
        label: "Freestyle",
    },
    Genre {
        id: 81,
        label: "Game",
    },
    Genre {
        id: 82,
        label: "Gangsta",
    },
    Genre {
        id: 83,
        label: "Gothic",
    },
    Genre {
        id: 84,
        label: "Gothic Rock",
    },
    Genre {
        id: 85,
        label: "House",
    },
    Genre {
        id: 86,
        label: "Humour",
    },
    Genre {
        id: 87,
        label: "Instrumental Pop",
    },
    Genre {
        id: 88,
        label: "Instrumental Rock",
    },
    Genre {
        id: 89,
        label: "Jazz Funk",
    },
    Genre {
        id: 90,
        label: "Jungle",
    },
    Genre {
        id: 91,
        label: "Lo-fi",
    },
    Genre {
        id: 92,
        label: "Meditative",
    },
    Genre {
        id: 93,
        label: "Musical",
    },
    Genre {
        id: 94,
        label: "National Folk",
    },
    Genre {
        id: 95,
        label: "Native American",
    },
    Genre {
        id: 96,
        label: "Noise",
    },
    Genre {
        id: 97,
        label: "Opera",
    },
    Genre {
        id: 98,
        label: "Polka",
    },
    Genre {
        id: 99,
        label: "Pop Folk",
    },
    Genre {
        id: 100,
        label: "Pop Funk",
    },
    Genre {
        id: 101,
        label: "Power Ballad",
    },
    Genre {
        id: 102,
        label: "Pranks",
    },
    Genre {
        id: 103,
        label: "Primus",
    },
    Genre {
        id: 104,
        label: "Psychedelic Rock",
    },
    Genre {
        id: 105,
        label: "Rave",
    },
    Genre {
        id: 106,
        label: "Retro",
    },
    Genre {
        id: 107,
        label: "Revival",
    },
    Genre {
        id: 108,
        label: "Rhythmic Soul",
    },
    Genre {
        id: 109,
        label: "Samba",
    },
    Genre {
        id: 110,
        label: "Satire",
    },
    Genre {
        id: 111,
        label: "Showtunes",
    },
    Genre {
        id: 112,
        label: "Slow Jam",
    },
    Genre {
        id: 113,
        label: "Slow Rock",
    },
    Genre {
        id: 114,
        label: "Sonata",
    },
    Genre {
        id: 115,
        label: "Sound Clip",
    },
    Genre {
        id: 116,
        label: "Space",
    },
    Genre {
        id: 117,
        label: "Speech",
    },
    Genre {
        id: 118,
        label: "Symphonic Rock",
    },
    Genre {
        id: 119,
        label: "Symphony",
    },
    Genre {
        id: 120,
        label: "Tango",
    },
    Genre {
        id: 121,
        label: "Techno Industrial",
    },
    Genre {
        id: 122,
        label: "Top40",
    },
    Genre {
        id: 123,
        label: "Trailer",
    },
    Genre {
        id: 124,
        label: "Trance",
    },
    Genre {
        id: 125,
        label: "Tribal",
    },
    Genre {
        id: 126,
        label: "Trip Hop",
    },
    Genre {
        id: 127,
        label: "Vocal",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PedalColor {
    pub id: i32,
    pub label: &'static str,
    pub rgb: [u8; 3],
}

pub const PEDAL_COLORS: &[PedalColor] = &[
    PedalColor {
        id: 0,
        label: "Aquamarine",
        rgb: [0x19, 0xcd, 0xd2],
    },
    PedalColor {
        id: 1,
        label: "Dark blue",
        rgb: [0x00, 0x7d, 0xff],
    },
    PedalColor {
        id: 2,
        label: "Dark green",
        rgb: [0x06, 0xa5, 0x5a],
    },
    PedalColor {
        id: 3,
        label: "Dark grey",
        rgb: [0x36, 0x39, 0x36],
    },
    PedalColor {
        id: 4,
        label: "Dark purple",
        rgb: [0x64, 0x27, 0xc5],
    },
    PedalColor {
        id: 5,
        label: "Dark red",
        rgb: [0xb4, 0x00, 0x00],
    },
    PedalColor {
        id: 6,
        label: "Light grey",
        rgb: [0x87, 0x8c, 0x87],
    },
    PedalColor {
        id: 7,
        label: "Light purple",
        rgb: [0xd6, 0x5b, 0xd5],
    },
    PedalColor {
        id: 8,
        label: "Lime green",
        rgb: [0x3d, 0xe2, 0x2d],
    },
    PedalColor {
        id: 9,
        label: "Medium blue",
        rgb: [0x00, 0xaa, 0xeb],
    },
    PedalColor {
        id: 10,
        label: "Orange",
        rgb: [0xff, 0x96, 0x00],
    },
    PedalColor {
        id: 11,
        label: "Red orange",
        rgb: [0xff, 0x7d, 0x00],
    },
    PedalColor {
        id: 12,
        label: "Rose red",
        rgb: [0xc3, 0x37, 0x41],
    },
    PedalColor {
        id: 13,
        label: "Violet",
        rgb: [0xc3, 0x3c, 0xbe],
    },
    PedalColor {
        id: 14,
        label: "Yellow",
        rgb: [0xff, 0xf4, 0x00],
    },
    PedalColor {
        id: 15,
        label: "Yellow orange",
        rgb: [0xf1, 0xb9, 0x00],
    },
];

/// Stompbox, modulation, delay, reverb respectively.
pub const DEFAULT_PEDAL_COLORS: [i32; 4] = [14, 1, 2, 10];
