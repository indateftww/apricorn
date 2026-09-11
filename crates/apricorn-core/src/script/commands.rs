//! The script command table — pret `src/data/fieldmap/script_cmd_table.h`
//! (`gScriptCmdTable`, 853 entries; `sNumScriptCmds` is its length) and
//! the operand layout of every command, from the assembler macros in
//! `asm/macros/script.inc` (each macro is `.short <opcode>` followed by
//! its `.byte`/`.short`/`.word` operands, which is exactly what the
//! `ScriptRead{Byte,Halfword,Word}` calls of the C handler consume).
//!
//! Only *names and sizes* live here — the table is code structure, not
//! game data. The names are pret's `ScrCmd_*` identifiers with the
//! prefix dropped (`ScrCmd_048` becomes [`Opcode::Cmd048`]; entry 149,
//! pret's unprefixed `UnsetPhoneCallTrigger`, keeps its name; the second
//! `ScrCmd_Dummy` at 486 is `Dummy486`, as pret's macro calls it).
//!
//! [`decode_at`] reads one instruction, [`disassemble`] walks a whole
//! bank by recursive descent from every entry point (following the
//! relative-branch operands `GoTo`/`Call`/`GoToIf`/`CallIf`/... and the
//! movement lists `ApplyMovement` points at), producing the per-opcode
//! histogram the ROM-gated inventory tests pin.

use std::collections::BTreeMap;

use super::ScriptError;
use super::bank::ScriptBank;

/// `sNumScriptCmds` — the number of table entries; an opcode at or
/// beyond it stops the script (`RunScriptCommand`, `src/script.c:76`).
pub const OPCODE_COUNT: usize = 853;

/// A movement-list step (`struct MovementScriptCommand`,
/// `include/unk_02062108.h:8`): a command and a repeat length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MovementCommand {
    /// The movement (`include/constants/movements.h`; 254 ends a list).
    pub command: u16,
    /// How many times it repeats.
    pub length: u16,
}

/// `MOVEMENT_STEP_END` — terminates a movement list.
pub const MOVEMENT_STEP_END: u16 = 254;

/// Every script command, by opcode (`gScriptCmdTable` index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u16)]
pub enum Opcode {
    /// `ScrCmd_Nop` (opcode 0).
    Nop = 0,
    /// `ScrCmd_Dummy` (opcode 1).
    Dummy = 1,
    /// `ScrCmd_End` (opcode 2).
    End = 2,
    /// `ScrCmd_Wait` (opcode 3).
    Wait = 3,
    /// `ScrCmd_LoadByte` (opcode 4).
    LoadByte = 4,
    /// `ScrCmd_LoadWord` (opcode 5).
    LoadWord = 5,
    /// `ScrCmd_LoadByteFromAddr` (opcode 6).
    LoadByteFromAddr = 6,
    /// `ScrCmd_WriteByteToAddr` (opcode 7).
    WriteByteToAddr = 7,
    /// `ScrCmd_SetPtrByte` (opcode 8).
    SetPtrByte = 8,
    /// `ScrCmd_CopyLocal` (opcode 9).
    CopyLocal = 9,
    /// `ScrCmd_CopyByte` (opcode 10).
    CopyByte = 10,
    /// `ScrCmd_CompareLocalToLocal` (opcode 11).
    CompareLocalToLocal = 11,
    /// `ScrCmd_CompareLocalToValue` (opcode 12).
    CompareLocalToValue = 12,
    /// `ScrCmd_CompareLocalToAddr` (opcode 13).
    CompareLocalToAddr = 13,
    /// `ScrCmd_CompareAddrToLocal` (opcode 14).
    CompareAddrToLocal = 14,
    /// `ScrCmd_CompareAddrToValue` (opcode 15).
    CompareAddrToValue = 15,
    /// `ScrCmd_CompareAddrToAddr` (opcode 16).
    CompareAddrToAddr = 16,
    /// `ScrCmd_CompareVarToValue` (opcode 17).
    CompareVarToValue = 17,
    /// `ScrCmd_CompareVarToVar` (opcode 18).
    CompareVarToVar = 18,
    /// `ScrCmd_RunScript` (opcode 19).
    RunScript = 19,
    /// `ScrCmd_CallStd` (opcode 20).
    CallStd = 20,
    /// `ScrCmd_RestartCurrentScript` (opcode 21).
    RestartCurrentScript = 21,
    /// `ScrCmd_GoTo` (opcode 22).
    GoTo = 22,
    /// `ScrCmd_ObjectGoTo` (opcode 23).
    ObjectGoTo = 23,
    /// `ScrCmd_BGGoTo` (opcode 24).
    BGGoTo = 24,
    /// `ScrCmd_DirectionGoTo` (opcode 25).
    DirectionGoTo = 25,
    /// `ScrCmd_Call` (opcode 26).
    Call = 26,
    /// `ScrCmd_Return` (opcode 27).
    Return = 27,
    /// `ScrCmd_GoToIf` (opcode 28).
    GoToIf = 28,
    /// `ScrCmd_CallIf` (opcode 29).
    CallIf = 29,
    /// `ScrCmd_SetFlag` (opcode 30).
    SetFlag = 30,
    /// `ScrCmd_ClearFlag` (opcode 31).
    ClearFlag = 31,
    /// `ScrCmd_CheckFlag` (opcode 32).
    CheckFlag = 32,
    /// `ScrCmd_SetFlagVar` (opcode 33).
    SetFlagVar = 33,
    /// `ScrCmd_ClearFlagVar` (opcode 34).
    ClearFlagVar = 34,
    /// `ScrCmd_CheckFlagVar` (opcode 35).
    CheckFlagVar = 35,
    /// `ScrCmd_SetTrainerFlag` (opcode 36).
    SetTrainerFlag = 36,
    /// `ScrCmd_ClearTrainerFlag` (opcode 37).
    ClearTrainerFlag = 37,
    /// `ScrCmd_CheckTrainerFlag` (opcode 38).
    CheckTrainerFlag = 38,
    /// `ScrCmd_AddVar` (opcode 39).
    AddVar = 39,
    /// `ScrCmd_SubVar` (opcode 40).
    SubVar = 40,
    /// `ScrCmd_SetVar` (opcode 41).
    SetVar = 41,
    /// `ScrCmd_CopyVar` (opcode 42).
    CopyVar = 42,
    /// `ScrCmd_SetOrCopyVar` (opcode 43).
    SetOrCopyVar = 43,
    /// `ScrCmd_NonNPCMsg` (opcode 44).
    NonNPCMsg = 44,
    /// `ScrCmd_NPCMsg` (opcode 45).
    NPCMsg = 45,
    /// `ScrCmd_NonNPCMsgVar` (opcode 46).
    NonNPCMsgVar = 46,
    /// `ScrCmd_NPCMsgVar` (opcode 47).
    NPCMsgVar = 47,
    /// `ScrCmd_048` (opcode 48).
    Cmd048 = 48,
    /// `ScrCmd_WaitABPress` (opcode 49).
    WaitABPress = 49,
    /// `ScrCmd_WaitButton` (opcode 50).
    WaitButton = 50,
    /// `ScrCmd_WaitButtonOrDpad` (opcode 51).
    WaitButtonOrDpad = 51,
    /// `ScrCmd_OpenMsg` (opcode 52).
    OpenMsg = 52,
    /// `ScrCmd_CloseMsg` (opcode 53).
    CloseMsg = 53,
    /// `ScrCmd_HoldMsg` (opcode 54).
    HoldMsg = 54,
    /// `ScrCmd_DirectionSignpost` (opcode 55).
    DirectionSignpost = 55,
    /// `ScrCmd_SetSignpostMap` (opcode 56).
    SetSignpostMap = 56,
    /// `ScrCmd_SetSignpostAction` (opcode 57).
    SetSignpostAction = 57,
    /// `ScrCmd_WaitSignpostAction` (opcode 58).
    WaitSignpostAction = 58,
    /// `ScrCmd_TrainerTips` (opcode 59).
    TrainerTips = 59,
    /// `ScrCmd_WaitSignpost` (opcode 60).
    WaitSignpost = 60,
    /// `ScrCmd_061` (opcode 61).
    Cmd061 = 61,
    /// `ScrCmd_062` (opcode 62).
    Cmd062 = 62,
    /// `ScrCmd_YesNo` (opcode 63).
    YesNo = 63,
    /// `ScrCmd_064` (opcode 64).
    Cmd064 = 64,
    /// `ScrCmd_065` (opcode 65).
    Cmd065 = 65,
    /// `ScrCmd_066` (opcode 66).
    Cmd066 = 66,
    /// `ScrCmd_067` (opcode 67).
    Cmd067 = 67,
    /// `ScrCmd_068` (opcode 68).
    Cmd068 = 68,
    /// `ScrCmd_069` (opcode 69).
    Cmd069 = 69,
    /// `ScrCmd_070` (opcode 70).
    Cmd070 = 70,
    /// `ScrCmd_071` (opcode 71).
    Cmd071 = 71,
    /// `ScrCmd_072` (opcode 72).
    Cmd072 = 72,
    /// `ScrCmd_PlaySE` (opcode 73).
    PlaySE = 73,
    /// `ScrCmd_StopSE` (opcode 74).
    StopSE = 74,
    /// `ScrCmd_WaitSE` (opcode 75).
    WaitSE = 75,
    /// `ScrCmd_PlayCry` (opcode 76).
    PlayCry = 76,
    /// `ScrCmd_WaitCry` (opcode 77).
    WaitCry = 77,
    /// `ScrCmd_PlayFanfare` (opcode 78).
    PlayFanfare = 78,
    /// `ScrCmd_WaitFanfare` (opcode 79).
    WaitFanfare = 79,
    /// `ScrCmd_PlayBGM` (opcode 80).
    PlayBGM = 80,
    /// `ScrCmd_StopBGM` (opcode 81).
    StopBGM = 81,
    /// `ScrCmd_ResetBGM` (opcode 82).
    ResetBGM = 82,
    /// `ScrCmd_083` (opcode 83).
    Cmd083 = 83,
    /// `ScrCmd_FadeOutBGM` (opcode 84).
    FadeOutBGM = 84,
    /// `ScrCmd_FadeInBGM` (opcode 85).
    FadeInBGM = 85,
    /// `ScrCmd_086` (opcode 86).
    Cmd086 = 86,
    /// `ScrCmd_TempBGM` (opcode 87).
    TempBGM = 87,
    /// `ScrCmd_088` (opcode 88).
    Cmd088 = 88,
    /// `ScrCmd_ChatotHasCry` (opcode 89).
    ChatotHasCry = 89,
    /// `ScrCmd_ChatotStartRecording` (opcode 90).
    ChatotStartRecording = 90,
    /// `ScrCmd_ChatotStopRecording` (opcode 91).
    ChatotStopRecording = 91,
    /// `ScrCmd_ChatotSaveRecording` (opcode 92).
    ChatotSaveRecording = 92,
    /// `ScrCmd_093` (opcode 93).
    Cmd093 = 93,
    /// `ScrCmd_ApplyMovement` (opcode 94).
    ApplyMovement = 94,
    /// `ScrCmd_WaitMovement` (opcode 95).
    WaitMovement = 95,
    /// `ScrCmd_LockAll` (opcode 96).
    LockAll = 96,
    /// `ScrCmd_ReleaseAll` (opcode 97).
    ReleaseAll = 97,
    /// `ScrCmd_Lock` (opcode 98).
    Lock = 98,
    /// `ScrCmd_Release` (opcode 99).
    Release = 99,
    /// `ScrCmd_ShowPerson` (opcode 100).
    ShowPerson = 100,
    /// `ScrCmd_HidePerson` (opcode 101).
    HidePerson = 101,
    /// `ScrCmd_102` (opcode 102).
    Cmd102 = 102,
    /// `ScrCmd_103` (opcode 103).
    Cmd103 = 103,
    /// `ScrCmd_FacePlayer` (opcode 104).
    FacePlayer = 104,
    /// `ScrCmd_GetPlayerCoords` (opcode 105).
    GetPlayerCoords = 105,
    /// `ScrCmd_GetPersonCoords` (opcode 106).
    GetPersonCoords = 106,
    /// `ScrCmd_107` (opcode 107).
    Cmd107 = 107,
    /// `ScrCmd_108` (opcode 108).
    Cmd108 = 108,
    /// `ScrCmd_109` (opcode 109).
    Cmd109 = 109,
    /// `ScrCmd_AddMoney` (opcode 110).
    AddMoney = 110,
    /// `ScrCmd_SubMoneyImmediate` (opcode 111).
    SubMoneyImmediate = 111,
    /// `ScrCmd_HasEnoughMoneyImmediate` (opcode 112).
    HasEnoughMoneyImmediate = 112,
    /// `ScrCmd_ShowMoneyBox` (opcode 113).
    ShowMoneyBox = 113,
    /// `ScrCmd_HideMoneyBox` (opcode 114).
    HideMoneyBox = 114,
    /// `ScrCmd_UpdateMoneyBox` (opcode 115).
    UpdateMoneyBox = 115,
    /// `ScrCmd_116` (opcode 116).
    Cmd116 = 116,
    /// `ScrCmd_117` (opcode 117).
    Cmd117 = 117,
    /// `ScrCmd_118` (opcode 118).
    Cmd118 = 118,
    /// `ScrCmd_GetCoinAmount` (opcode 119).
    GetCoinAmount = 119,
    /// `ScrCmd_GiveCoins` (opcode 120).
    GiveCoins = 120,
    /// `ScrCmd_TakeCoins` (opcode 121).
    TakeCoins = 121,
    /// `ScrCmd_GiveAthletePoints` (opcode 122).
    GiveAthletePoints = 122,
    /// `ScrCmd_TakeAthletePoints` (opcode 123).
    TakeAthletePoints = 123,
    /// `ScrCmd_CheckAthletePoints` (opcode 124).
    CheckAthletePoints = 124,
    /// `ScrCmd_GiveItem` (opcode 125).
    GiveItem = 125,
    /// `ScrCmd_TakeItem` (opcode 126).
    TakeItem = 126,
    /// `ScrCmd_HasSpaceForItem` (opcode 127).
    HasSpaceForItem = 127,
    /// `ScrCmd_HasItem` (opcode 128).
    HasItem = 128,
    /// `ScrCmd_ItemIsTMOrHM` (opcode 129).
    ItemIsTMOrHM = 129,
    /// `ScrCmd_GetItemPocket` (opcode 130).
    GetItemPocket = 130,
    /// `ScrCmd_SetStarterChoice` (opcode 131).
    SetStarterChoice = 131,
    /// `ScrCmd_GenderMsgBox` (opcode 132).
    GenderMsgBox = 132,
    /// `ScrCmd_GetSealQuantity` (opcode 133).
    GetSealQuantity = 133,
    /// `ScrCmd_GiveOrTakeSeal` (opcode 134).
    GiveOrTakeSeal = 134,
    /// `ScrCmd_GiveRandomSeal` (opcode 135).
    GiveRandomSeal = 135,
    /// `ScrCmd_136` (opcode 136).
    Cmd136 = 136,
    /// `ScrCmd_GiveMon` (opcode 137).
    GiveMon = 137,
    /// `ScrCmd_GiveEgg` (opcode 138).
    GiveEgg = 138,
    /// `ScrCmd_SetMonMove` (opcode 139).
    SetMonMove = 139,
    /// `ScrCmd_MonHasMove` (opcode 140).
    MonHasMove = 140,
    /// `ScrCmd_GetPartySlotWithMove` (opcode 141).
    GetPartySlotWithMove = 141,
    /// `ScrCmd_GetPhoneBookRematch` (opcode 142).
    GetPhoneBookRematch = 142,
    /// `ScrCmd_NameRival` (opcode 143).
    NameRival = 143,
    /// `ScrCmd_GetFriendSprite` (opcode 144).
    GetFriendSprite = 144,
    /// `ScrCmd_RegisterPokegearCard` (opcode 145).
    RegisterPokegearCard = 145,
    /// `ScrCmd_RegisterGearNumber` (opcode 146).
    RegisterGearNumber = 146,
    /// `ScrCmd_CheckRegisteredPhoneNumber` (opcode 147).
    CheckRegisteredPhoneNumber = 147,
    /// `ScrCmd_148` (opcode 148).
    Cmd148 = 148,
    /// `UnsetPhoneCallTrigger` (opcode 149).
    UnsetPhoneCallTrigger = 149,
    /// `ScrCmd_RestoreOverworld` (opcode 150).
    RestoreOverworld = 150,
    /// `ScrCmd_151` (opcode 151).
    Cmd151 = 151,
    /// `ScrCmd_152` (opcode 152).
    Cmd152 = 152,
    /// `ScrCmd_153` (opcode 153).
    Cmd153 = 153,
    /// `ScrCmd_154` (opcode 154).
    Cmd154 = 154,
    /// `ScrCmd_155` (opcode 155).
    Cmd155 = 155,
    /// `ScrCmd_156` (opcode 156).
    Cmd156 = 156,
    /// `ScrCmd_TownMap` (opcode 157).
    TownMap = 157,
    /// `ScrCmd_158` (opcode 158).
    Cmd158 = 158,
    /// `ScrCmd_159` (opcode 159).
    Cmd159 = 159,
    /// `ScrCmd_160` (opcode 160).
    Cmd160 = 160,
    /// `ScrCmd_161` (opcode 161).
    Cmd161 = 161,
    /// `ScrCmd_162` (opcode 162).
    Cmd162 = 162,
    /// `ScrCmd_HOFCredits` (opcode 163).
    HOFCredits = 163,
    /// `ScrCmd_164` (opcode 164).
    Cmd164 = 164,
    /// `ScrCmd_165` (opcode 165).
    Cmd165 = 165,
    /// `ScrCmd_166` (opcode 166).
    Cmd166 = 166,
    /// `ScrCmd_ChooseStarter` (opcode 167).
    ChooseStarter = 167,
    /// `ScrCmd_GetTrainerPathToPlayer` (opcode 168).
    GetTrainerPathToPlayer = 168,
    /// `ScrCmd_TrainerStepTowardsPlayer` (opcode 169).
    TrainerStepTowardsPlayer = 169,
    /// `ScrCmd_GetTrainerEyeType` (opcode 170).
    GetTrainerEyeType = 170,
    /// `ScrCmd_GetEyeTrainerNum` (opcode 171).
    GetEyeTrainerNum = 171,
    /// `ScrCmd_NamePlayer` (opcode 172).
    NamePlayer = 172,
    /// `ScrCmd_NicknameInput` (opcode 173).
    NicknameInput = 173,
    /// `ScrCmd_FadeScreen` (opcode 174).
    FadeScreen = 174,
    /// `ScrCmd_WaitFade` (opcode 175).
    WaitFade = 175,
    /// `ScrCmd_Warp` (opcode 176).
    Warp = 176,
    /// `ScrCmd_RockClimb` (opcode 177).
    RockClimb = 177,
    /// `ScrCmd_Surf` (opcode 178).
    Surf = 178,
    /// `ScrCmd_Waterfall` (opcode 179).
    Waterfall = 179,
    /// `ScrCmd_180` (opcode 180).
    Cmd180 = 180,
    /// `ScrCmd_FlashEffect` (opcode 181).
    FlashEffect = 181,
    /// `ScrCmd_Whirlpool` (opcode 182).
    Whirlpool = 182,
    /// `ScrCmd_183` (opcode 183).
    Cmd183 = 183,
    /// `ScrCmd_PlayerOnBikeCheck` (opcode 184).
    PlayerOnBikeCheck = 184,
    /// `ScrCmd_PlayerOnBikeSet` (opcode 185).
    PlayerOnBikeSet = 185,
    /// `ScrCmd_SetBikeStateLock` (opcode 186).
    SetBikeStateLock = 186,
    /// `ScrCmd_GetPlayerState` (opcode 187).
    GetPlayerState = 187,
    /// `ScrCmd_SetAvatarBits` (opcode 188).
    SetAvatarBits = 188,
    /// `ScrCmd_UpdateAvatarState` (opcode 189).
    UpdateAvatarState = 189,
    /// `ScrCmd_BufferPlayersName` (opcode 190).
    BufferPlayersName = 190,
    /// `ScrCmd_BufferRivalsName` (opcode 191).
    BufferRivalsName = 191,
    /// `ScrCmd_BufferFriendsName` (opcode 192).
    BufferFriendsName = 192,
    /// `ScrCmd_BufferMonSpeciesName` (opcode 193).
    BufferMonSpeciesName = 193,
    /// `ScrCmd_BufferItemName` (opcode 194).
    BufferItemName = 194,
    /// `ScrCmd_BufferPocketName` (opcode 195).
    BufferPocketName = 195,
    /// `ScrCmd_BufferTMHMMoveName` (opcode 196).
    BufferTMHMMoveName = 196,
    /// `ScrCmd_BufferMoveName` (opcode 197).
    BufferMoveName = 197,
    /// `ScrCmd_BufferInt` (opcode 198).
    BufferInt = 198,
    /// `ScrCmd_BufferPartyMonNick` (opcode 199).
    BufferPartyMonNick = 199,
    /// `ScrCmd_BufferTrainerClassName` (opcode 200).
    BufferTrainerClassName = 200,
    /// `ScrCmd_BufferPlayerUnionAvatarClassName` (opcode 201).
    BufferPlayerUnionAvatarClassName = 201,
    /// `ScrCmd_BufferSpeciesName` (opcode 202).
    BufferSpeciesName = 202,
    /// `ScrCmd_BufferStarterSpeciesName` (opcode 203).
    BufferStarterSpeciesName = 203,
    /// `ScrCmd_BufferDPPtRivalStarterSpeciesName` (opcode 204).
    BufferDPPtRivalStarterSpeciesName = 204,
    /// `ScrCmd_BufferDPPtFriendStarterSpeciesName` (opcode 205).
    BufferDPPtFriendStarterSpeciesName = 205,
    /// `ScrCmd_GetStarterChoice` (opcode 206).
    GetStarterChoice = 206,
    /// `ScrCmd_BufferDecorationName` (opcode 207).
    BufferDecorationName = 207,
    /// `ScrCmd_208` (opcode 208).
    Cmd208 = 208,
    /// `ScrCmd_209` (opcode 209).
    Cmd209 = 209,
    /// `ScrCmd_BufferMapSecName` (opcode 210).
    BufferMapSecName = 210,
    /// `ScrCmd_211` (opcode 211).
    Cmd211 = 211,
    /// `ScrCmd_GetTrainerNum` (opcode 212).
    GetTrainerNum = 212,
    /// `ScrCmd_TrainerBattle` (opcode 213).
    TrainerBattle = 213,
    /// `ScrCmd_TrainerMessage` (opcode 214).
    TrainerMessage = 214,
    /// `ScrCmd_GetTrainerMsgParams` (opcode 215).
    GetTrainerMsgParams = 215,
    /// `ScrCmd_GetRematchMsgParams` (opcode 216).
    GetRematchMsgParams = 216,
    /// `ScrCmd_TrainerIsDoubleBattle` (opcode 217).
    TrainerIsDoubleBattle = 217,
    /// `ScrCmd_EncounterMusic` (opcode 218).
    EncounterMusic = 218,
    /// `ScrCmd_WhiteOut` (opcode 219).
    WhiteOut = 219,
    /// `ScrCmd_CheckBattleWon` (opcode 220).
    CheckBattleWon = 220,
    /// `ScrCmd_StaticWildWonOrCaughtCheck` (opcode 221).
    StaticWildWonOrCaughtCheck = 221,
    /// `ScrCmd_PartyCheckForDouble` (opcode 222).
    PartyCheckForDouble = 222,
    /// `ScrCmd_223` (opcode 223).
    Cmd223 = 223,
    /// `ScrCmd_224` (opcode 224).
    Cmd224 = 224,
    /// `ScrCmd_GoToIfTrainerDefeated` (opcode 225).
    GoToIfTrainerDefeated = 225,
    /// `ScrCmd_226` (opcode 226).
    Cmd226 = 226,
    /// `ScrCmd_227` (opcode 227).
    Cmd227 = 227,
    /// `ScrCmd_228` (opcode 228).
    Cmd228 = 228,
    /// `ScrCmd_229` (opcode 229).
    Cmd229 = 229,
    /// `ScrCmd_230` (opcode 230).
    Cmd230 = 230,
    /// `ScrCmd_231` (opcode 231).
    Cmd231 = 231,
    /// `ScrCmd_232` (opcode 232).
    Cmd232 = 232,
    /// `ScrCmd_233` (opcode 233).
    Cmd233 = 233,
    /// `ScrCmd_234` (opcode 234).
    Cmd234 = 234,
    /// `ScrCmd_235` (opcode 235).
    Cmd235 = 235,
    /// `ScrCmd_236` (opcode 236).
    Cmd236 = 236,
    /// `ScrCmd_237` (opcode 237).
    Cmd237 = 237,
    /// `ScrCmd_PartyHasPokerus` (opcode 238).
    PartyHasPokerus = 238,
    /// `ScrCmd_MonGetGender` (opcode 239).
    MonGetGender = 239,
    /// `ScrCmd_SetDynamicWarp` (opcode 240).
    SetDynamicWarp = 240,
    /// `ScrCmd_GetDynamicWarpFloorNo` (opcode 241).
    GetDynamicWarpFloorNo = 241,
    /// `ScrCmd_ElevatorCurFloorBox` (opcode 242).
    ElevatorCurFloorBox = 242,
    /// `ScrCmd_CountJohtoDexSeen` (opcode 243).
    CountJohtoDexSeen = 243,
    /// `ScrCmd_CountJohtoDexOwned` (opcode 244).
    CountJohtoDexOwned = 244,
    /// `ScrCmd_CountNationalDexSeen` (opcode 245).
    CountNationalDexSeen = 245,
    /// `ScrCmd_CountNationalDexOwned` (opcode 246).
    CountNationalDexOwned = 246,
    /// `ScrCmd_247` (opcode 247).
    Cmd247 = 247,
    /// `ScrCmd_GetDexEvalResult` (opcode 248).
    GetDexEvalResult = 248,
    /// `ScrCmd_RocketTrapBattle` (opcode 249).
    RocketTrapBattle = 249,
    /// `ScrCmd_250` (opcode 250).
    Cmd250 = 250,
    /// `ScrCmd_CatchingTutorial` (opcode 251).
    CatchingTutorial = 251,
    /// `ScrCmd_252` (opcode 252).
    Cmd252 = 252,
    /// `ScrCmd_GetSaveFileState` (opcode 253).
    GetSaveFileState = 253,
    /// `ScrCmd_SaveGameNormal` (opcode 254).
    SaveGameNormal = 254,
    /// `ScrCmd_255` (opcode 255).
    Cmd255 = 255,
    /// `ScrCmd_256` (opcode 256).
    Cmd256 = 256,
    /// `ScrCmd_257` (opcode 257).
    Cmd257 = 257,
    /// `ScrCmd_258` (opcode 258).
    Cmd258 = 258,
    /// `ScrCmd_259` (opcode 259).
    Cmd259 = 259,
    /// `ScrCmd_260` (opcode 260).
    Cmd260 = 260,
    /// `ScrCmd_261` (opcode 261).
    Cmd261 = 261,
    /// `ScrCmd_262` (opcode 262).
    Cmd262 = 262,
    /// `ScrCmd_263` (opcode 263).
    Cmd263 = 263,
    /// `ScrCmd_264` (opcode 264).
    Cmd264 = 264,
    /// `ScrCmd_265` (opcode 265).
    Cmd265 = 265,
    /// `ScrCmd_266` (opcode 266).
    Cmd266 = 266,
    /// `ScrCmd_267` (opcode 267).
    Cmd267 = 267,
    /// `ScrCmd_268` (opcode 268).
    Cmd268 = 268,
    /// `ScrCmd_269` (opcode 269).
    Cmd269 = 269,
    /// `ScrCmd_270` (opcode 270).
    Cmd270 = 270,
    /// `ScrCmd_271` (opcode 271).
    Cmd271 = 271,
    /// `ScrCmd_272` (opcode 272).
    Cmd272 = 272,
    /// `ScrCmd_273` (opcode 273).
    Cmd273 = 273,
    /// `ScrCmd_274` (opcode 274).
    Cmd274 = 274,
    /// `ScrCmd_MartBuy` (opcode 275).
    MartBuy = 275,
    /// `ScrCmd_SpecialMartBuy` (opcode 276).
    SpecialMartBuy = 276,
    /// `ScrCmd_DecorationMart` (opcode 277).
    DecorationMart = 277,
    /// `ScrCmd_SealMart` (opcode 278).
    SealMart = 278,
    /// `ScrCmd_OverworldWhiteOut` (opcode 279).
    OverworldWhiteOut = 279,
    /// `ScrCmd_SetSpawn` (opcode 280).
    SetSpawn = 280,
    /// `ScrCmd_GetPlayerGender` (opcode 281).
    GetPlayerGender = 281,
    /// `ScrCmd_HealParty` (opcode 282).
    HealParty = 282,
    /// `ScrCmd_283` (opcode 283).
    Cmd283 = 283,
    /// `ScrCmd_284` (opcode 284).
    Cmd284 = 284,
    /// `ScrCmd_285` (opcode 285).
    Cmd285 = 285,
    /// `ScrCmd_286` (opcode 286).
    Cmd286 = 286,
    /// `ScrCmd_BufferUnionRoomAvatarChoices` (opcode 287).
    BufferUnionRoomAvatarChoices = 287,
    /// `ScrCmd_UnionRoomAvatarIdxToTrainerClass` (opcode 288).
    UnionRoomAvatarIdxToTrainerClass = 288,
    /// `ScrCmd_289` (opcode 289).
    Cmd289 = 289,
    /// `ScrCmd_CheckPokedex` (opcode 290).
    CheckPokedex = 290,
    /// `ScrCmd_GivePokedex` (opcode 291).
    GivePokedex = 291,
    /// `ScrCmd_CheckRunningShoes` (opcode 292).
    CheckRunningShoes = 292,
    /// `ScrCmd_GiveRunningShoes` (opcode 293).
    GiveRunningShoes = 293,
    /// `ScrCmd_CheckBadge` (opcode 294).
    CheckBadge = 294,
    /// `ScrCmd_GiveBadge` (opcode 295).
    GiveBadge = 295,
    /// `ScrCmd_CountBadges` (opcode 296).
    CountBadges = 296,
    /// `ScrCmd_297` (opcode 297).
    Cmd297 = 297,
    /// `ScrCmd_298` (opcode 298).
    Cmd298 = 298,
    /// `ScrCmd_CheckEscortMode` (opcode 299).
    CheckEscortMode = 299,
    /// `ScrCmd_SetEscortMode` (opcode 300).
    SetEscortMode = 300,
    /// `ScrCmd_ClearEscortMode` (opcode 301).
    ClearEscortMode = 301,
    /// `ScrCmd_CheckStepTakenFlag` (opcode 302).
    CheckStepTakenFlag = 302,
    /// `ScrCmd_SetStepTakenFlag` (opcode 303).
    SetStepTakenFlag = 303,
    /// `ScrCmd_GetStepTakenFlag` (opcode 304).
    GetStepTakenFlag = 304,
    /// `ScrCmd_CheckGameClearFlag` (opcode 305).
    CheckGameClearFlag = 305,
    /// `ScrCmd_SetGameClearFlag` (opcode 306).
    SetGameClearFlag = 306,
    /// `ScrCmd_307` (opcode 307).
    Cmd307 = 307,
    /// `ScrCmd_308` (opcode 308).
    Cmd308 = 308,
    /// `ScrCmd_309` (opcode 309).
    Cmd309 = 309,
    /// `ScrCmd_310` (opcode 310).
    Cmd310 = 310,
    /// `ScrCmd_311` (opcode 311).
    Cmd311 = 311,
    /// `ScrCmd_BufferDaycareMonNicks` (opcode 312).
    BufferDaycareMonNicks = 312,
    /// `ScrCmd_GetDaycareState` (opcode 313).
    GetDaycareState = 313,
    /// `ScrCmd_EcruteakGymInit` (opcode 314).
    EcruteakGymInit = 314,
    /// `ScrCmd_315` (opcode 315).
    Cmd315 = 315,
    /// `ScrCmd_316` (opcode 316).
    Cmd316 = 316,
    /// `ScrCmd_317` (opcode 317).
    Cmd317 = 317,
    /// `ScrCmd_CianwoodGymInit` (opcode 318).
    CianwoodGymInit = 318,
    /// `ScrCmd_CianwoodGymTurnWinch` (opcode 319).
    CianwoodGymTurnWinch = 319,
    /// `ScrCmd_VermilionGymInit` (opcode 320).
    VermilionGymInit = 320,
    /// `ScrCmd_VermilionGymLockAction` (opcode 321).
    VermilionGymLockAction = 321,
    /// `ScrCmd_VermilionGymCanCheck` (opcode 322).
    VermilionGymCanCheck = 322,
    /// `ScrCmd_ResampleVermilionGymCans` (opcode 323).
    ResampleVermilionGymCans = 323,
    /// `ScrCmd_VioletGymInit` (opcode 324).
    VioletGymInit = 324,
    /// `ScrCmd_VioletGymElevator` (opcode 325).
    VioletGymElevator = 325,
    /// `ScrCmd_AzaleaGymInit` (opcode 326).
    AzaleaGymInit = 326,
    /// `ScrCmd_AzaleaGymSpinarak` (opcode 327).
    AzaleaGymSpinarak = 327,
    /// `ScrCmd_AzaleaGymSwitch` (opcode 328).
    AzaleaGymSwitch = 328,
    /// `ScrCmd_BlackthornGymInit` (opcode 329).
    BlackthornGymInit = 329,
    /// `ScrCmd_FuchsiaGymInit` (opcode 330).
    FuchsiaGymInit = 330,
    /// `ScrCmd_ViridianGymInit` (opcode 331).
    ViridianGymInit = 331,
    /// `ScrCmd_GetPartyCount` (opcode 332).
    GetPartyCount = 332,
    /// `ScrCmd_333` (opcode 333).
    Cmd333 = 333,
    /// `ScrCmd_334` (opcode 334).
    Cmd334 = 334,
    /// `ScrCmd_335` (opcode 335).
    Cmd335 = 335,
    /// `ScrCmd_BufferBerryName` (opcode 336).
    BufferBerryName = 336,
    /// `ScrCmd_BufferNatureName` (opcode 337).
    BufferNatureName = 337,
    /// `ScrCmd_MovePerson` (opcode 338).
    MovePerson = 338,
    /// `ScrCmd_MovePersonFacing` (opcode 339).
    MovePersonFacing = 339,
    /// `ScrCmd_SetObjectMovementType` (opcode 340).
    SetObjectMovementType = 340,
    /// `ScrCmd_SetObjectFacing` (opcode 341).
    SetObjectFacing = 341,
    /// `ScrCmd_MoveWarp` (opcode 342).
    MoveWarp = 342,
    /// `ScrCmd_MoveBGEvent` (opcode 343).
    MoveBGEvent = 343,
    /// `ScrCmd_344` (opcode 344).
    Cmd344 = 344,
    /// `ScrCmd_AddWaitingIcon` (opcode 345).
    AddWaitingIcon = 345,
    /// `ScrCmd_RemoveWaitingIcon` (opcode 346).
    RemoveWaitingIcon = 346,
    /// `ScrCmd_347` (opcode 347).
    Cmd347 = 347,
    /// `ScrCmd_WaitButtonOrDelay` (opcode 348).
    WaitButtonOrDelay = 348,
    /// `ScrCmd_PartySelectUI` (opcode 349).
    PartySelectUI = 349,
    /// `ScrCmd_350` (opcode 350).
    Cmd350 = 350,
    /// `ScrCmd_GetPartySelection` (opcode 351).
    GetPartySelection = 351,
    /// `ScrCmd_PokemonSummaryScreen` (opcode 352).
    PokemonSummaryScreen = 352,
    /// `ScrCmd_GetMoveSelection` (opcode 353).
    GetMoveSelection = 353,
    /// `ScrCmd_GetPartyMonSpecies` (opcode 354).
    GetPartyMonSpecies = 354,
    /// `ScrCmd_PartyMonIsMine` (opcode 355).
    PartyMonIsMine = 355,
    /// `ScrCmd_PartyCountNotEgg` (opcode 356).
    PartyCountNotEgg = 356,
    /// `ScrCmd_CountAliveMons` (opcode 357).
    CountAliveMons = 357,
    /// `ScrCmd_CountAliveMonsAndPC` (opcode 358).
    CountAliveMonsAndPC = 358,
    /// `ScrCmd_PartyCountEgg` (opcode 359).
    PartyCountEgg = 359,
    /// `ScrCmd_SubMoneyVar` (opcode 360).
    SubMoneyVar = 360,
    /// `ScrCmd_RetrieveDaycareMon` (opcode 361).
    RetrieveDaycareMon = 361,
    /// `ScrCmd_GiveLoanMon` (opcode 362).
    GiveLoanMon = 362,
    /// `ScrCmd_CheckReturnLoanMon` (opcode 363).
    CheckReturnLoanMon = 363,
    /// `ScrCmd_ReturnLoanMon` (opcode 364).
    ReturnLoanMon = 364,
    /// `ScrCmd_ResetDaycareEgg` (opcode 365).
    ResetDaycareEgg = 365,
    /// `ScrCmd_GiveDaycareEgg` (opcode 366).
    GiveDaycareEgg = 366,
    /// `ScrCmd_BufferDaycareWithdrawCost` (opcode 367).
    BufferDaycareWithdrawCost = 367,
    /// `ScrCmd_HasEnoughMoneyVar` (opcode 368).
    HasEnoughMoneyVar = 368,
    /// `ScrCmd_EggHatchAnim` (opcode 369).
    EggHatchAnim = 369,
    /// `ScrCmd_370` (opcode 370).
    Cmd370 = 370,
    /// `ScrCmd_BufferDaycareMonGrowth` (opcode 371).
    BufferDaycareMonGrowth = 371,
    /// `ScrCmd_GetTailDaycareMonSpeciesAndNick` (opcode 372).
    GetTailDaycareMonSpeciesAndNick = 372,
    /// `ScrCmd_PutMonInDaycare` (opcode 373).
    PutMonInDaycare = 373,
    /// `ScrCmd_374` (opcode 374).
    Cmd374 = 374,
    /// `ScrCmd_MakeObjectVisible` (opcode 375).
    MakeObjectVisible = 375,
    /// `ScrCmd_376` (opcode 376).
    Cmd376 = 376,
    /// `ScrCmd_377` (opcode 377).
    Cmd377 = 377,
    /// `ScrCmd_ViewRankings` (opcode 378).
    ViewRankings = 378,
    /// `ScrCmd_379` (opcode 379).
    Cmd379 = 379,
    /// `ScrCmd_Random` (opcode 380).
    Random = 380,
    /// `ScrCmd_381` (opcode 381).
    Cmd381 = 381,
    /// `ScrCmd_MonGetFriendship` (opcode 382).
    MonGetFriendship = 382,
    /// `ScrCmd_MonAddFriendship` (opcode 383).
    MonAddFriendship = 383,
    /// `ScrCmd_MonSubtractFriendship` (opcode 384).
    MonSubtractFriendship = 384,
    /// `ScrCmd_BufferDaycareMonStats` (opcode 385).
    BufferDaycareMonStats = 385,
    /// `ScrCmd_GetPlayerFacing` (opcode 386).
    GetPlayerFacing = 386,
    /// `ScrCmd_GetDaycareCompatibility` (opcode 387).
    GetDaycareCompatibility = 387,
    /// `ScrCmd_CheckDaycareEgg` (opcode 388).
    CheckDaycareEgg = 388,
    /// `ScrCmd_PlayerHasSpecies` (opcode 389).
    PlayerHasSpecies = 389,
    /// `ScrCmd_SizeRecordCompare` (opcode 390).
    SizeRecordCompare = 390,
    /// `ScrCmd_SizeRecordUpdate` (opcode 391).
    SizeRecordUpdate = 391,
    /// `ScrCmd_BufferMonSize` (opcode 392).
    BufferMonSize = 392,
    /// `ScrCmd_BufferRecordSize` (opcode 393).
    BufferRecordSize = 393,
    /// `ScrCmd_394` (opcode 394).
    Cmd394 = 394,
    /// `ScrCmd_395` (opcode 395).
    Cmd395 = 395,
    /// `ScrCmd_CountMonMoves` (opcode 396).
    CountMonMoves = 396,
    /// `ScrCmd_MonForgetMove` (opcode 397).
    MonForgetMove = 397,
    /// `ScrCmd_MonGetMove` (opcode 398).
    MonGetMove = 398,
    /// `ScrCmd_BufferPartyMonMoveName` (opcode 399).
    BufferPartyMonMoveName = 399,
    /// `ScrCmd_StrengthFlagAction` (opcode 400).
    StrengthFlagAction = 400,
    /// `ScrCmd_FlashAction` (opcode 401).
    FlashAction = 401,
    /// `ScrCmd_DefogAction` (opcode 402).
    DefogAction = 402,
    /// `ScrCmd_403` (opcode 403).
    Cmd403 = 403,
    /// `ScrCmd_404` (opcode 404).
    Cmd404 = 404,
    /// `ScrCmd_405` (opcode 405).
    Cmd405 = 405,
    /// `ScrCmd_406` (opcode 406).
    Cmd406 = 406,
    /// `ScrCmd_407` (opcode 407).
    Cmd407 = 407,
    /// `ScrCmd_408` (opcode 408).
    Cmd408 = 408,
    /// `ScrCmd_409` (opcode 409).
    Cmd409 = 409,
    /// `ScrCmd_410` (opcode 410).
    Cmd410 = 410,
    /// `ScrCmd_411` (opcode 411).
    Cmd411 = 411,
    /// `ScrCmd_412` (opcode 412).
    Cmd412 = 412,
    /// `ScrCmd_413` (opcode 413).
    Cmd413 = 413,
    /// `ScrCmd_414` (opcode 414).
    Cmd414 = 414,
    /// `ScrCmd_415` (opcode 415).
    Cmd415 = 415,
    /// `ScrCmd_416` (opcode 416).
    Cmd416 = 416,
    /// `ScrCmd_417` (opcode 417).
    Cmd417 = 417,
    /// `ScrCmd_418` (opcode 418).
    Cmd418 = 418,
    /// `ScrCmd_419` (opcode 419).
    Cmd419 = 419,
    /// `ScrCmd_420` (opcode 420).
    Cmd420 = 420,
    /// `ScrCmd_421` (opcode 421).
    Cmd421 = 421,
    /// `ScrCmd_422` (opcode 422).
    Cmd422 = 422,
    /// `ScrCmd_CheckJohtoDexComplete` (opcode 423).
    CheckJohtoDexComplete = 423,
    /// `ScrCmd_CheckNationalDexComplete` (opcode 424).
    CheckNationalDexComplete = 424,
    /// `ScrCmd_ShowCertificate` (opcode 425).
    ShowCertificate = 425,
    /// `ScrCmd_KenyaCheck` (opcode 426).
    KenyaCheck = 426,
    /// `ScrCmd_427` (opcode 427).
    Cmd427 = 427,
    /// `ScrCmd_MonGiveMail` (opcode 428).
    MonGiveMail = 428,
    /// `ScrCmd_CountFossils` (opcode 429).
    CountFossils = 429,
    /// `ScrCmd_SetPhoneCall` (opcode 430).
    SetPhoneCall = 430,
    /// `ScrCmd_RunPhoneCall` (opcode 431).
    RunPhoneCall = 431,
    /// `ScrCmd_GetFossilPokemon` (opcode 432).
    GetFossilPokemon = 432,
    /// `ScrCmd_GetFossilMinimumAmount` (opcode 433).
    GetFossilMinimumAmount = 433,
    /// `ScrCmd_PartyCountMonsAtOrBelowLevel` (opcode 434).
    PartyCountMonsAtOrBelowLevel = 434,
    /// `ScrCmd_SurvivePoisoning` (opcode 435).
    SurvivePoisoning = 435,
    /// `ScrCmd_436` (opcode 436).
    Cmd436 = 436,
    /// `ScrCmd_DebugWatch` (opcode 437).
    DebugWatch = 437,
    /// `ScrCmd_GetStdMsgNaix` (opcode 438).
    GetStdMsgNaix = 438,
    /// `ScrCmd_NonNPCMsgExtern` (opcode 439).
    NonNPCMsgExtern = 439,
    /// `ScrCmd_MsgBoxExtern` (opcode 440).
    MsgBoxExtern = 440,
    /// `ScrCmd_441` (opcode 441).
    Cmd441 = 441,
    /// `ScrCmd_442` (opcode 442).
    Cmd442 = 442,
    /// `ScrCmd_443` (opcode 443).
    Cmd443 = 443,
    /// `ScrCmd_444` (opcode 444).
    Cmd444 = 444,
    /// `ScrCmd_445` (opcode 445).
    Cmd445 = 445,
    /// `ScrCmd_446` (opcode 446).
    Cmd446 = 446,
    /// `ScrCmd_SafariZoneAction` (opcode 447).
    SafariZoneAction = 447,
    /// `ScrCmd_448` (opcode 448).
    Cmd448 = 448,
    /// `ScrCmd_449` (opcode 449).
    Cmd449 = 449,
    /// `ScrCmd_450` (opcode 450).
    Cmd450 = 450,
    /// `ScrCmd_451` (opcode 451).
    Cmd451 = 451,
    /// `ScrCmd_452` (opcode 452).
    Cmd452 = 452,
    /// `ScrCmd_453` (opcode 453).
    Cmd453 = 453,
    /// `ScrCmd_454` (opcode 454).
    Cmd454 = 454,
    /// `ScrCmd_455` (opcode 455).
    Cmd455 = 455,
    /// `ScrCmd_456` (opcode 456).
    Cmd456 = 456,
    /// `ScrCmd_MonGetNature` (opcode 457).
    MonGetNature = 457,
    /// `ScrCmd_GetPartySlotWithNature` (opcode 458).
    GetPartySlotWithNature = 458,
    /// `ScrCmd_459` (opcode 459).
    Cmd459 = 459,
    /// `ScrCmd_LoadPhoneDat` (opcode 460).
    LoadPhoneDat = 460,
    /// `ScrCmd_GetPhoneContactMsgIds` (opcode 461).
    GetPhoneContactMsgIds = 461,
    /// `ScrCmd_462` (opcode 462).
    Cmd462 = 462,
    /// `ScrCmd_EnableMassOutbreaks` (opcode 463).
    EnableMassOutbreaks = 463,
    /// `ScrCmd_CreateRoamer` (opcode 464).
    CreateRoamer = 464,
    /// `ScrCmd_465` (opcode 465).
    Cmd465 = 465,
    /// `ScrCmd_466` (opcode 466).
    Cmd466 = 466,
    /// `ScrCmd_MoveRelearnerInit` (opcode 467).
    MoveRelearnerInit = 467,
    /// `ScrCmd_MoveTutorInit` (opcode 468).
    MoveTutorInit = 468,
    /// `ScrCmd_MoveRelearnerGetResult` (opcode 469).
    MoveRelearnerGetResult = 469,
    /// `ScrCmd_LoadNPCTrade` (opcode 470).
    LoadNPCTrade = 470,
    /// `ScrCmd_GetOfferedSpecies` (opcode 471).
    GetOfferedSpecies = 471,
    /// `ScrCmd_NPCTradeGetReqSpecies` (opcode 472).
    NPCTradeGetReqSpecies = 472,
    /// `ScrCmd_NPCTradeExec` (opcode 473).
    NPCTradeExec = 473,
    /// `ScrCmd_NPCTradeEnd` (opcode 474).
    NPCTradeEnd = 474,
    /// `ScrCmd_475` (opcode 475).
    Cmd475 = 475,
    /// `ScrCmd_EnablePokedexFormDetection` (opcode 476).
    EnablePokedexFormDetection = 476,
    /// `ScrCmd_NatDexFlagAction` (opcode 477).
    NatDexFlagAction = 477,
    /// `ScrCmd_MonGetRibbonCount` (opcode 478).
    MonGetRibbonCount = 478,
    /// `ScrCmd_GetPartyRibbonCount` (opcode 479).
    GetPartyRibbonCount = 479,
    /// `ScrCmd_MonHasRibbon` (opcode 480).
    MonHasRibbon = 480,
    /// `ScrCmd_GiveRibbon` (opcode 481).
    GiveRibbon = 481,
    /// `ScrCmd_BufferRibbonName` (opcode 482).
    BufferRibbonName = 482,
    /// `ScrCmd_GetEVTotal` (opcode 483).
    GetEVTotal = 483,
    /// `ScrCmd_GetWeekday` (opcode 484).
    GetWeekday = 484,
    /// `ScrCmd_StartBattleRegulationMenuTask` (opcode 485).
    StartBattleRegulationMenuTask = 485,
    /// `ScrCmd_Dummy` (opcode 486).
    Dummy486 = 486,
    /// `ScrCmd_PokeCenAnim` (opcode 487).
    PokeCenAnim = 487,
    /// `ScrCmd_ElevatorAnim` (opcode 488).
    ElevatorAnim = 488,
    /// `ScrCmd_MysteryGift` (opcode 489).
    MysteryGift = 489,
    /// `ScrCmd_NopVar490` (opcode 490).
    NopVar490 = 490,
    /// `ScrCmd_491` (opcode 491).
    Cmd491 = 491,
    /// `ScrCmd_492` (opcode 492).
    Cmd492 = 492,
    /// `ScrCmd_PromptEasyChat` (opcode 493).
    PromptEasyChat = 493,
    /// `ScrCmd_494` (opcode 494).
    Cmd494 = 494,
    /// `ScrCmd_GetGameVersion` (opcode 495).
    GetGameVersion = 495,
    /// `ScrCmd_GetPartyLead` (opcode 496).
    GetPartyLead = 496,
    /// `ScrCmd_GetMonTypes` (opcode 497).
    GetMonTypes = 497,
    /// `ScrCmd_PrimoPasswordCheck1` (opcode 498).
    PrimoPasswordCheck1 = 498,
    /// `ScrCmd_PrimoPasswordCheck2` (opcode 499).
    PrimoPasswordCheck2 = 499,
    /// `ScrCmd_500` (opcode 500).
    Cmd500 = 500,
    /// `ScrCmd_501` (opcode 501).
    Cmd501 = 501,
    /// `ScrCmd_502` (opcode 502).
    Cmd502 = 502,
    /// `ScrCmd_LotoIDGet` (opcode 503).
    LotoIDGet = 503,
    /// `ScrCmd_LotoIDSearch` (opcode 504).
    LotoIDSearch = 504,
    /// `ScrCmd_LotoIDSet` (opcode 505).
    LotoIDSet = 505,
    /// `ScrCmd_BufferBoxMonNick` (opcode 506).
    BufferBoxMonNick = 506,
    /// `ScrCmd_CountPCEmptySpace` (opcode 507).
    CountPCEmptySpace = 507,
    /// `ScrCmd_PalParkAction` (opcode 508).
    PalParkAction = 508,
    /// `ScrCmd_509` (opcode 509).
    Cmd509 = 509,
    /// `ScrCmd_510` (opcode 510).
    Cmd510 = 510,
    /// `ScrCmd_PalParkScoreGet` (opcode 511).
    PalParkScoreGet = 511,
    /// `ScrCmd_PlayerMovementSavingSet` (opcode 512).
    PlayerMovementSavingSet = 512,
    /// `ScrCmd_PlayerMovementSavingClear` (opcode 513).
    PlayerMovementSavingClear = 513,
    /// `ScrCmd_HallOfFameAnim` (opcode 514).
    HallOfFameAnim = 514,
    /// `ScrCmd_AddSpecialGameStat` (opcode 515).
    AddSpecialGameStat = 515,
    /// `ScrCmd_BufferFashionName` (opcode 516).
    BufferFashionName = 516,
    /// `ScrCmd_517` (opcode 517).
    Cmd517 = 517,
    /// `ScrCmd_518` (opcode 518).
    Cmd518 = 518,
    /// `ScrCmd_519` (opcode 519).
    Cmd519 = 519,
    /// `ScrCmd_520` (opcode 520).
    Cmd520 = 520,
    /// `ScrCmd_521` (opcode 521).
    Cmd521 = 521,
    /// `ScrCmd_522` (opcode 522).
    Cmd522 = 522,
    /// `ScrCmd_523` (opcode 523).
    Cmd523 = 523,
    /// `ScrCmd_524` (opcode 524).
    Cmd524 = 524,
    /// `ScrCmd_525` (opcode 525).
    Cmd525 = 525,
    /// `ScrCmd_526` (opcode 526).
    Cmd526 = 526,
    /// `ScrCmd_527` (opcode 527).
    Cmd527 = 527,
    /// `ScrCmd_528` (opcode 528).
    Cmd528 = 528,
    /// `ScrCmd_GetPartyLeadAlive` (opcode 529).
    GetPartyLeadAlive = 529,
    /// `ScrCmd_530` (opcode 530).
    Cmd530 = 530,
    /// `ScrCmd_BufferBackgroundName` (opcode 531).
    BufferBackgroundName = 531,
    /// `ScrCmd_CheckCoinsImmediate` (opcode 532).
    CheckCoinsImmediate = 532,
    /// `ScrCmd_CheckGiveCoins` (opcode 533).
    CheckGiveCoins = 533,
    /// `ScrCmd_534` (opcode 534).
    Cmd534 = 534,
    /// `ScrCmd_MonGetLevel` (opcode 535).
    MonGetLevel = 535,
    /// `ScrCmd_536` (opcode 536).
    Cmd536 = 536,
    /// `ScrCmd_537` (opcode 537).
    Cmd537 = 537,
    /// `ScrCmd_538` (opcode 538).
    Cmd538 = 538,
    /// `ScrCmd_539` (opcode 539).
    Cmd539 = 539,
    /// `ScrCmd_540` (opcode 540).
    Cmd540 = 540,
    /// `ScrCmd_BufferIntEx` (opcode 541).
    BufferIntEx = 541,
    /// `ScrCmd_MonGetContestValue` (opcode 542).
    MonGetContestValue = 542,
    /// `ScrCmd_543` (opcode 543).
    Cmd543 = 543,
    /// `ScrCmd_544` (opcode 544).
    Cmd544 = 544,
    /// `ScrCmd_545` (opcode 545).
    Cmd545 = 545,
    /// `ScrCmd_546` (opcode 546).
    Cmd546 = 546,
    /// `ScrCmd_547` (opcode 547).
    Cmd547 = 547,
    /// `ScrCmd_548` (opcode 548).
    Cmd548 = 548,
    /// `ScrCmd_549` (opcode 549).
    Cmd549 = 549,
    /// `ScrCmd_550` (opcode 550).
    Cmd550 = 550,
    /// `ScrCmd_551` (opcode 551).
    Cmd551 = 551,
    /// `ScrCmd_552` (opcode 552).
    Cmd552 = 552,
    /// `ScrCmd_553` (opcode 553).
    Cmd553 = 553,
    /// `ScrCmd_554` (opcode 554).
    Cmd554 = 554,
    /// `ScrCmd_555` (opcode 555).
    Cmd555 = 555,
    /// `ScrCmd_556` (opcode 556).
    Cmd556 = 556,
    /// `ScrCmd_CheckBattlePoints` (opcode 557).
    CheckBattlePoints = 557,
    /// `ScrCmd_UnionRoomAvatarIdxToSprite` (opcode 558).
    UnionRoomAvatarIdxToSprite = 558,
    /// `ScrCmd_559` (opcode 559).
    Cmd559 = 559,
    /// `ScrCmd_560` (opcode 560).
    Cmd560 = 560,
    /// `ScrCmd_ScreenShake` (opcode 561).
    ScreenShake = 561,
    /// `ScrCmd_MultiBattle` (opcode 562).
    MultiBattle = 562,
    /// `ScrCmd_563` (opcode 563).
    Cmd563 = 563,
    /// `ScrCmd_564` (opcode 564).
    Cmd564 = 564,
    /// `ScrCmd_565` (opcode 565).
    Cmd565 = 565,
    /// `ScrCmd_566` (opcode 566).
    Cmd566 = 566,
    /// `ScrCmd_GetDPPlPrizeItemIDAndCost` (opcode 567).
    GetDPPlPrizeItemIDAndCost = 567,
    /// `ScrCmd_568` (opcode 568).
    Cmd568 = 568,
    /// `ScrCmd_569` (opcode 569).
    Cmd569 = 569,
    /// `ScrCmd_CheckCoinsVar` (opcode 570).
    CheckCoinsVar = 570,
    /// `ScrCmd_571` (opcode 571).
    Cmd571 = 571,
    /// `ScrCmd_GetUniqueSealsQuantity` (opcode 572).
    GetUniqueSealsQuantity = 572,
    /// `ScrCmd_573` (opcode 573).
    Cmd573 = 573,
    /// `ScrCmd_574` (opcode 574).
    Cmd574 = 574,
    /// `ScrCmd_575` (opcode 575).
    Cmd575 = 575,
    /// `ScrCmd_576` (opcode 576).
    Cmd576 = 576,
    /// `ScrCmd_577` (opcode 577).
    Cmd577 = 577,
    /// `ScrCmd_578` (opcode 578).
    Cmd578 = 578,
    /// `ScrCmd_579` (opcode 579).
    Cmd579 = 579,
    /// `ScrCmd_BufferSealName` (opcode 580).
    BufferSealName = 580,
    /// `ScrCmd_LockLastTalked` (opcode 581).
    LockLastTalked = 581,
    /// `ScrCmd_582` (opcode 582).
    Cmd582 = 582,
    /// `ScrCmd_583` (opcode 583).
    Cmd583 = 583,
    /// `ScrCmd_PartyLegalCheck` (opcode 584).
    PartyLegalCheck = 584,
    /// `ScrCmd_585` (opcode 585).
    Cmd585 = 585,
    /// `ScrCmd_586` (opcode 586).
    Cmd586 = 586,
    /// `ScrCmd_587` (opcode 587).
    Cmd587 = 587,
    /// `ScrCmd_LatiCaughtCheck` (opcode 588).
    LatiCaughtCheck = 588,
    /// `ScrCmd_WildBattle` (opcode 589).
    WildBattle = 589,
    /// `ScrCmd_GetTrcardStars` (opcode 590).
    GetTrcardStars = 590,
    /// `ScrCmd_591` (opcode 591).
    Cmd591 = 591,
    /// `ScrCmd_592` (opcode 592).
    Cmd592 = 592,
    /// `ScrCmd_ShowSaveStats` (opcode 593).
    ShowSaveStats = 593,
    /// `ScrCmd_HideSaveStats` (opcode 594).
    HideSaveStats = 594,
    /// `ScrCmd_595` (opcode 595).
    Cmd595 = 595,
    /// `ScrCmd_596` (opcode 596).
    Cmd596 = 596,
    /// `ScrCmd_597` (opcode 597).
    Cmd597 = 597,
    /// `ScrCmd_598` (opcode 598).
    Cmd598 = 598,
    /// `ScrCmd_599` (opcode 599).
    Cmd599 = 599,
    /// `ScrCmd_600` (opcode 600).
    Cmd600 = 600,
    /// `ScrCmd_FollowMonFacePlayer` (opcode 601).
    FollowMonFacePlayer = 601,
    /// `ScrCmd_ToggleFollowingPokemonMovement` (opcode 602).
    ToggleFollowingPokemonMovement = 602,
    /// `ScrCmd_WaitFollowingPokemonMovement` (opcode 603).
    WaitFollowingPokemonMovement = 603,
    /// `ScrCmd_FollowingPokemonMovement` (opcode 604).
    FollowingPokemonMovement = 604,
    /// `ScrCmd_605` (opcode 605).
    Cmd605 = 605,
    /// `ScrCmd_606` (opcode 606).
    Cmd606 = 606,
    /// `ScrCmd_607` (opcode 607).
    Cmd607 = 607,
    /// `ScrCmd_608` (opcode 608).
    Cmd608 = 608,
    /// `ScrCmd_609` (opcode 609).
    Cmd609 = 609,
    /// `ScrCmd_610` (opcode 610).
    Cmd610 = 610,
    /// `ScrCmd_Pokeathlon` (opcode 611).
    Pokeathlon = 611,
    /// `ScrCmd_GetNPCTradeUnusedFlag` (opcode 612).
    GetNPCTradeUnusedFlag = 612,
    /// `ScrCmd_GetPhoneContactRandomGiftBerry` (opcode 613).
    GetPhoneContactRandomGiftBerry = 613,
    /// `ScrCmd_GetPhoneContactGiftItem` (opcode 614).
    GetPhoneContactGiftItem = 614,
    /// `ScrCmd_CameronPhoto` (opcode 615).
    CameronPhoto = 615,
    /// `ScrCmd_CountSavedPhotos` (opcode 616).
    CountSavedPhotos = 616,
    /// `ScrCmd_OpenPhotoAlbum` (opcode 617).
    OpenPhotoAlbum = 617,
    /// `ScrCmd_PhotoAlbumIsFull` (opcode 618).
    PhotoAlbumIsFull = 618,
    /// `ScrCmd_RocketCostumeFlagCheck` (opcode 619).
    RocketCostumeFlagCheck = 619,
    /// `ScrCmd_RocketCostumeFlagAction` (opcode 620).
    RocketCostumeFlagAction = 620,
    /// `ScrCmd_PlaceStarterBallsInElmsLab` (opcode 621).
    PlaceStarterBallsInElmsLab = 621,
    /// `ScrCmd_622` (opcode 622).
    Cmd622 = 622,
    /// `ScrCmd_AnimApricornTree` (opcode 623).
    AnimApricornTree = 623,
    /// `ScrCmd_ApricornTreeGetApricorn` (opcode 624).
    ApricornTreeGetApricorn = 624,
    /// `ScrCmd_GiveApricornFromTree` (opcode 625).
    GiveApricornFromTree = 625,
    /// `ScrCmd_BufferApricornName` (opcode 626).
    BufferApricornName = 626,
    /// `ScrCmd_627` (opcode 627).
    Cmd627 = 627,
    /// `ScrCmd_628` (opcode 628).
    Cmd628 = 628,
    /// `ScrCmd_629` (opcode 629).
    Cmd629 = 629,
    /// `ScrCmd_630` (opcode 630).
    Cmd630 = 630,
    /// `ScrCmd_631` (opcode 631).
    Cmd631 = 631,
    /// `ScrCmd_CountPartyMonsOfSpecies` (opcode 632).
    CountPartyMonsOfSpecies = 632,
    /// `ScrCmd_633` (opcode 633).
    Cmd633 = 633,
    /// `ScrCmd_634` (opcode 634).
    Cmd634 = 634,
    /// `ScrCmd_635` (opcode 635).
    Cmd635 = 635,
    /// `ScrCmd_636` (opcode 636).
    Cmd636 = 636,
    /// `ScrCmd_637` (opcode 637).
    Cmd637 = 637,
    /// `ScrCmd_638` (opcode 638).
    Cmd638 = 638,
    /// `ScrCmd_639` (opcode 639).
    Cmd639 = 639,
    /// `ScrCmd_640` (opcode 640).
    Cmd640 = 640,
    /// `ScrCmd_SaveWipeExtraChunks` (opcode 641).
    SaveWipeExtraChunks = 641,
    /// `ScrCmd_642` (opcode 642).
    Cmd642 = 642,
    /// `ScrCmd_643` (opcode 643).
    Cmd643 = 643,
    /// `ScrCmd_644` (opcode 644).
    Cmd644 = 644,
    /// `ScrCmd_645` (opcode 645).
    Cmd645 = 645,
    /// `ScrCmd_646` (opcode 646).
    Cmd646 = 646,
    /// `ScrCmd_GetPartySlotWithSpecies` (opcode 647).
    GetPartySlotWithSpecies = 647,
    /// `ScrCmd_648` (opcode 648).
    Cmd648 = 648,
    /// `ScrCmd_ScratchOffCard` (opcode 649).
    ScratchOffCard = 649,
    /// `ScrCmd_ScratchOffCardEnd` (opcode 650).
    ScratchOffCardEnd = 650,
    /// `ScrCmd_GetScratchOffPrize` (opcode 651).
    GetScratchOffPrize = 651,
    /// `ScrCmd_652` (opcode 652).
    Cmd652 = 652,
    /// `ScrCmd_MoveTutorChooseMove` (opcode 653).
    MoveTutorChooseMove = 653,
    /// `ScrCmd_TutorMoveTeachInSlot` (opcode 654).
    TutorMoveTeachInSlot = 654,
    /// `ScrCmd_TutorMoveGetPrice` (opcode 655).
    TutorMoveGetPrice = 655,
    /// `ScrCmd_656` (opcode 656).
    Cmd656 = 656,
    /// `ScrCmd_StatJudge` (opcode 657).
    StatJudge = 657,
    /// `ScrCmd_BufferStatName` (opcode 658).
    BufferStatName = 658,
    /// `ScrCmd_SetMonForm` (opcode 659).
    SetMonForm = 659,
    /// `ScrCmd_BufferTrainerName` (opcode 660).
    BufferTrainerName = 660,
    /// `ScrCmd_661` (opcode 661).
    Cmd661 = 661,
    /// `ScrCmd_662` (opcode 662).
    Cmd662 = 662,
    /// `ScrCmd_663` (opcode 663).
    Cmd663 = 663,
    /// `ScrCmd_664` (opcode 664).
    Cmd664 = 664,
    /// `ScrCmd_665` (opcode 665).
    Cmd665 = 665,
    /// `ScrCmd_666` (opcode 666).
    Cmd666 = 666,
    /// `ScrCmd_667` (opcode 667).
    Cmd667 = 667,
    /// `ScrCmd_BufferTypeName` (opcode 668).
    BufferTypeName = 668,
    /// `ScrCmd_GetItemQuantity` (opcode 669).
    GetItemQuantity = 669,
    /// `ScrCmd_GetHiddenPowerType` (opcode 670).
    GetHiddenPowerType = 670,
    /// `ScrCmd_SetFavoriteMon` (opcode 671).
    SetFavoriteMon = 671,
    /// `ScrCmd_GetFavoriteMon` (opcode 672).
    GetFavoriteMon = 672,
    /// `ScrCmd_GetOwnedRotomForms` (opcode 673).
    GetOwnedRotomForms = 673,
    /// `ScrCmd_CountTranformedRotomsInParty` (opcode 674).
    CountTranformedRotomsInParty = 674,
    /// `ScrCmd_UpdateRotomForm` (opcode 675).
    UpdateRotomForm = 675,
    /// `ScrCmd_GetPartyMonForm` (opcode 676).
    GetPartyMonForm = 676,
    /// `ScrCmd_677` (opcode 677).
    Cmd677 = 677,
    /// `ScrCmd_678` (opcode 678).
    Cmd678 = 678,
    /// `ScrCmd_679` (opcode 679).
    Cmd679 = 679,
    /// `ScrCmd_AddSpecialGameStat2` (opcode 680).
    AddSpecialGameStat2 = 680,
    /// `ScrCmd_681` (opcode 681).
    Cmd681 = 681,
    /// `ScrCmd_682` (opcode 682).
    Cmd682 = 682,
    /// `ScrCmd_GetStaticEncounterOutcome` (opcode 683).
    GetStaticEncounterOutcome = 683,
    /// `ScrCmd_684` (opcode 684).
    Cmd684 = 684,
    /// `ScrCmd_GetPlayerXYZ` (opcode 685).
    GetPlayerXYZ = 685,
    /// `ScrCmd_686` (opcode 686).
    Cmd686 = 686,
    /// `ScrCmd_687` (opcode 687).
    Cmd687 = 687,
    /// `ScrCmd_GetPartySlotWithFatefulEncounter` (opcode 688).
    GetPartySlotWithFatefulEncounter = 688,
    /// `ScrCmd_CommSanitizeParty` (opcode 689).
    CommSanitizeParty = 689,
    /// `ScrCmd_DaycareSanitizeMon` (opcode 690).
    DaycareSanitizeMon = 690,
    /// `ScrCmd_691` (opcode 691).
    Cmd691 = 691,
    /// `ScrCmd_BufferBattleHallStreak` (opcode 692).
    BufferBattleHallStreak = 692,
    /// `ScrCmd_BattleHallCountUsedSpecies` (opcode 693).
    BattleHallCountUsedSpecies = 693,
    /// `ScrCmd_BattleHallGetTotalStreak` (opcode 694).
    BattleHallGetTotalStreak = 694,
    /// `ScrCmd_695` (opcode 695).
    Cmd695 = 695,
    /// `ScrCmd_696` (opcode 696).
    Cmd696 = 696,
    /// `ScrCmd_697` (opcode 697).
    Cmd697 = 697,
    /// `ScrCmd_FollowerPokeIsEventTrigger` (opcode 698).
    FollowerPokeIsEventTrigger = 698,
    /// `ScrCmd_699` (opcode 699).
    Cmd699 = 699,
    /// `ScrCmd_700` (opcode 700).
    Cmd700 = 700,
    /// `ScrCmd_MonHasItem` (opcode 701).
    MonHasItem = 701,
    /// `ScrCmd_BattleTowerSetUpMultiBattle` (opcode 702).
    BattleTowerSetUpMultiBattle = 702,
    /// `ScrCmd_SetPlayerVolume` (opcode 703).
    SetPlayerVolume = 703,
    /// `ScrCmd_704` (opcode 704).
    Cmd704 = 704,
    /// `ScrCmd_705` (opcode 705).
    Cmd705 = 705,
    /// `ScrCmd_706` (opcode 706).
    Cmd706 = 706,
    /// `ScrCmd_CheckMonSeen` (opcode 707).
    CheckMonSeen = 707,
    /// `ScrCmd_708` (opcode 708).
    Cmd708 = 708,
    /// `ScrCmd_709` (opcode 709).
    Cmd709 = 709,
    /// `ScrCmd_710` (opcode 710).
    Cmd710 = 710,
    /// `ScrCmd_FollowMonInteract` (opcode 711).
    FollowMonInteract = 711,
    /// `ScrCmd_712` (opcode 712).
    Cmd712 = 712,
    /// `ScrCmd_AlphPuzzle` (opcode 713).
    AlphPuzzle = 713,
    /// `ScrCmd_OpenAlphHiddenRoom` (opcode 714).
    OpenAlphHiddenRoom = 714,
    /// `ScrCmd_UpdateDaycareMonObjects` (opcode 715).
    UpdateDaycareMonObjects = 715,
    /// `ScrCmd_716` (opcode 716).
    Cmd716 = 716,
    /// `ScrCmd_717` (opcode 717).
    Cmd717 = 717,
    /// `ScrCmd_718` (opcode 718).
    Cmd718 = 718,
    /// `ScrCmd_719` (opcode 719).
    Cmd719 = 719,
    /// `ScrCmd_720` (opcode 720).
    Cmd720 = 720,
    /// `ScrCmd_721` (opcode 721).
    Cmd721 = 721,
    /// `ScrCmd_722` (opcode 722).
    Cmd722 = 722,
    /// `ScrCmd_723` (opcode 723).
    Cmd723 = 723,
    /// `ScrCmd_724` (opcode 724).
    Cmd724 = 724,
    /// `ScrCmd_725` (opcode 725).
    Cmd725 = 725,
    /// `ScrCmd_ProcessSoundplate` (opcode 726).
    ProcessSoundplate = 726,
    /// `ScrCmd_GetFollowPokePartyIndex` (opcode 727).
    GetFollowPokePartyIndex = 727,
    /// `ScrCmd_728` (opcode 728).
    Cmd728 = 728,
    /// `ScrCmd_729` (opcode 729).
    Cmd729 = 729,
    /// `ScrCmd_730` (opcode 730).
    Cmd730 = 730,
    /// `ScrCmd_731` (opcode 731).
    Cmd731 = 731,
    /// `ScrCmd_732` (opcode 732).
    Cmd732 = 732,
    /// `ScrCmd_733` (opcode 733).
    Cmd733 = 733,
    /// `ScrCmd_734` (opcode 734).
    Cmd734 = 734,
    /// `ScrCmd_735` (opcode 735).
    Cmd735 = 735,
    /// `ScrCmd_ClearKurtApricorn` (opcode 736).
    ClearKurtApricorn = 736,
    /// `ScrCmd_737` (opcode 737).
    Cmd737 = 737,
    /// `ScrCmd_GetTotalApricornCount` (opcode 738).
    GetTotalApricornCount = 738,
    /// `ScrCmd_739` (opcode 739).
    Cmd739 = 739,
    /// `ScrCmd_740` (opcode 740).
    Cmd740 = 740,
    /// `ScrCmd_741` (opcode 741).
    Cmd741 = 741,
    /// `ScrCmd_742` (opcode 742).
    Cmd742 = 742,
    /// `ScrCmd_743` (opcode 743).
    Cmd743 = 743,
    /// `ScrCmd_CreatePokeathlonFriendshipRoomStatues` (opcode 744).
    CreatePokeathlonFriendshipRoomStatues = 744,
    /// `ScrCmd_BufferPokeathlonCourseName` (opcode 745).
    BufferPokeathlonCourseName = 745,
    /// `ScrCmd_TouchscreenMenuHide` (opcode 746).
    TouchscreenMenuHide = 746,
    /// `ScrCmd_TouchscreenMenuShow` (opcode 747).
    TouchscreenMenuShow = 747,
    /// `ScrCmd_GetMenuChoice` (opcode 748).
    GetMenuChoice = 748,
    /// `ScrCmd_MenuInitStdGmm` (opcode 749).
    MenuInitStdGmm = 749,
    /// `ScrCmd_MenuInit` (opcode 750).
    MenuInit = 750,
    /// `ScrCmd_MenuItemAdd` (opcode 751).
    MenuItemAdd = 751,
    /// `ScrCmd_MenuExec` (opcode 752).
    MenuExec = 752,
    /// `ScrCmd_RockSmashItemCheck` (opcode 753).
    RockSmashItemCheck = 753,
    /// `ScrCmd_TryHeadbuttEncounter` (opcode 754).
    TryHeadbuttEncounter = 754,
    /// `ScrCmd_LegendCutsceneClearBellAnimBegin` (opcode 755).
    LegendCutsceneClearBellAnimBegin = 755,
    /// `ScrCmd_LegendCutsceneClearBellAnimEnd` (opcode 756).
    LegendCutsceneClearBellAnimEnd = 756,
    /// `ScrCmd_LegendCutsceneClearBellRiseFromBag` (opcode 757).
    LegendCutsceneClearBellRiseFromBag = 757,
    /// `ScrCmd_LegendCutsceneClearBellShimmer` (opcode 758).
    LegendCutsceneClearBellShimmer = 758,
    /// `ScrCmd_LegendCutsceneLugiaEyeGlimmerEffect` (opcode 759).
    LegendCutsceneLugiaEyeGlimmerEffect = 759,
    /// `ScrCmd_760` (opcode 760).
    Cmd760 = 760,
    /// `ScrCmd_LegendCutsceneMoveCameraTo` (opcode 761).
    LegendCutsceneMoveCameraTo = 761,
    /// `ScrCmd_LegendCutscenePanCameraTo` (opcode 762).
    LegendCutscenePanCameraTo = 762,
    /// `ScrCmd_LegendCutsceneWaitCameraPan` (opcode 763).
    LegendCutsceneWaitCameraPan = 763,
    /// `ScrCmd_LegendCutsceneBirdFinalApproach` (opcode 764).
    LegendCutsceneBirdFinalApproach = 764,
    /// `ScrCmd_LegendCutsceneWavesOrLeavesEffectBegin` (opcode 765).
    LegendCutsceneWavesOrLeavesEffectBegin = 765,
    /// `ScrCmd_LegendCutsceneWavesOrLeavesEffectEnd` (opcode 766).
    LegendCutsceneWavesOrLeavesEffectEnd = 766,
    /// `ScrCmd_LegendCutsceneLugiaArrivesEffectBegin` (opcode 767).
    LegendCutsceneLugiaArrivesEffectBegin = 767,
    /// `ScrCmd_LegendCutsceneLugiaArrivesEffectEnd` (opcode 768).
    LegendCutsceneLugiaArrivesEffectEnd = 768,
    /// `ScrCmd_LegendCutsceneLugiaArrivesEffectCameraPan` (opcode 769).
    LegendCutsceneLugiaArrivesEffectCameraPan = 769,
    /// `ScrCmd_CheckSeenAllLetterUnown` (opcode 770).
    CheckSeenAllLetterUnown = 770,
    /// `ScrCmd_771` (opcode 771).
    Cmd771 = 771,
    /// `ScrCmd_772` (opcode 772).
    Cmd772 = 772,
    /// `ScrCmd_Cinematic` (opcode 773).
    Cinematic = 773,
    /// `ScrCmd_ShowLegendaryWing` (opcode 774).
    ShowLegendaryWing = 774,
    /// `ScrCmd_775` (opcode 775).
    Cmd775 = 775,
    /// `ScrCmd_GiveTogepiEgg` (opcode 776).
    GiveTogepiEgg = 776,
    /// `ScrCmd_777` (opcode 777).
    Cmd777 = 777,
    /// `ScrCmd_GiveSpikyEarPichu` (opcode 778).
    GiveSpikyEarPichu = 778,
    /// `ScrCmd_RadioMusicIsPlaying` (opcode 779).
    RadioMusicIsPlaying = 779,
    /// `ScrCmd_CasinoGame` (opcode 780).
    CasinoGame = 780,
    /// `ScrCmd_KenyaCheckPartyOrMailbox` (opcode 781).
    KenyaCheckPartyOrMailbox = 781,
    /// `ScrCmd_MartSell` (opcode 782).
    MartSell = 782,
    /// `ScrCmd_SetFollowMonInhibitState` (opcode 783).
    SetFollowMonInhibitState = 783,
    /// `ScrCmd_ScriptOverlayCmd` (opcode 784).
    ScriptOverlayCmd = 784,
    /// `ScrCmd_BugContestAction` (opcode 785).
    BugContestAction = 785,
    /// `ScrCmd_BufferBugContestWinner` (opcode 786).
    BufferBugContestWinner = 786,
    /// `ScrCmd_JudgeBugContest` (opcode 787).
    JudgeBugContest = 787,
    /// `ScrCmd_BufferBugContestMonNick` (opcode 788).
    BufferBugContestMonNick = 788,
    /// `ScrCmd_BugContestGetTimeLeft` (opcode 789).
    BugContestGetTimeLeft = 789,
    /// `ScrCmd_IsBugContestantRegistered` (opcode 790).
    IsBugContestantRegistered = 790,
    /// `ScrCmd_CheckSafariZoneChallengeCompleted` (opcode 791).
    CheckSafariZoneChallengeCompleted = 791,
    /// `ScrCmd_UpdateSafariZoneIGT` (opcode 792).
    UpdateSafariZoneIGT = 792,
    /// `ScrCmd_BankTransaction` (opcode 793).
    BankTransaction = 793,
    /// `ScrCmd_CheckBankBalance` (opcode 794).
    CheckBankBalance = 794,
    /// `ScrCmd_795` (opcode 795).
    Cmd795 = 795,
    /// `ScrCmd_796` (opcode 796).
    Cmd796 = 796,
    /// `ScrCmd_797` (opcode 797).
    Cmd797 = 797,
    /// `ScrCmd_BufferRulesetName` (opcode 798).
    BufferRulesetName = 798,
    /// `ScrCmd_799` (opcode 799).
    Cmd799 = 799,
    /// `ScrCmd_800` (opcode 800).
    Cmd800 = 800,
    /// `ScrCmd_801` (opcode 801).
    Cmd801 = 801,
    /// `ScrCmd_802` (opcode 802).
    Cmd802 = 802,
    /// `ScrCmd_803` (opcode 803).
    Cmd803 = 803,
    /// `ScrCmd_804` (opcode 804).
    Cmd804 = 804,
    /// `ScrCmd_805` (opcode 805).
    Cmd805 = 805,
    /// `ScrCmd_806` (opcode 806).
    Cmd806 = 806,
    /// `ScrCmd_SetTrainerHouseSprite` (opcode 807).
    SetTrainerHouseSprite = 807,
    /// `ScrCmd_808` (opcode 808).
    Cmd808 = 808,
    /// `ScrCmd_ShowTrainerHouseIntroMessage` (opcode 809).
    ShowTrainerHouseIntroMessage = 809,
    /// `ScrCmd_810` (opcode 810).
    Cmd810 = 810,
    /// `ScrCmd_811` (opcode 811).
    Cmd811 = 811,
    /// `ScrCmd_812` (opcode 812).
    Cmd812 = 812,
    /// `ScrCmd_MomGiftCheck` (opcode 813).
    MomGiftCheck = 813,
    /// `ScrCmd_814` (opcode 814).
    Cmd814 = 814,
    /// `ScrCmd_815` (opcode 815).
    Cmd815 = 815,
    /// `ScrCmd_UnownCircle` (opcode 816).
    UnownCircle = 816,
    /// `ScrCmd_817` (opcode 817).
    Cmd817 = 817,
    /// `ScrCmd_MystriStageGymmickInit` (opcode 818).
    MystriStageGymmickInit = 818,
    /// `ScrCmd_819` (opcode 819).
    Cmd819 = 819,
    /// `ScrCmd_820` (opcode 820).
    Cmd820 = 820,
    /// `ScrCmd_GetBuenasPassword` (opcode 821).
    GetBuenasPassword = 821,
    /// `ScrCmd_822` (opcode 822).
    Cmd822 = 822,
    /// `ScrCmd_823` (opcode 823).
    Cmd823 = 823,
    /// `ScrCmd_824` (opcode 824).
    Cmd824 = 824,
    /// `ScrCmd_GetShinyLeafCount` (opcode 825).
    GetShinyLeafCount = 825,
    /// `ScrCmd_TryGiveShinyLeafCrown` (opcode 826).
    TryGiveShinyLeafCrown = 826,
    /// `ScrCmd_GetPartyMonForm2` (opcode 827).
    GetPartyMonForm2 = 827,
    /// `ScrCmd_MonAddContestValue` (opcode 828).
    MonAddContestValue = 828,
    /// `ScrCmd_829` (opcode 829).
    Cmd829 = 829,
    /// `ScrCmd_830` (opcode 830).
    Cmd830 = 830,
    /// `ScrCmd_831` (opcode 831).
    Cmd831 = 831,
    /// `ScrCmd_832` (opcode 832).
    Cmd832 = 832,
    /// `ScrCmd_833` (opcode 833).
    Cmd833 = 833,
    /// `ScrCmd_834` (opcode 834).
    Cmd834 = 834,
    /// `ScrCmd_835` (opcode 835).
    Cmd835 = 835,
    /// `ScrCmd_CheckKyogreGroudonInParty` (opcode 836).
    CheckKyogreGroudonInParty = 836,
    /// `ScrCmd_837` (opcode 837).
    Cmd837 = 837,
    /// `ScrCmd_BankOrWalletIsFull` (opcode 838).
    BankOrWalletIsFull = 838,
    /// `ScrCmd_SysSetSleepFlag` (opcode 839).
    SysSetSleepFlag = 839,
    /// `ScrCmd_840` (opcode 840).
    Cmd840 = 840,
    /// `ScrCmd_841` (opcode 841).
    Cmd841 = 841,
    /// `ScrCmd_842` (opcode 842).
    Cmd842 = 842,
    /// `ScrCmd_BufferItemNameIndef` (opcode 843).
    BufferItemNameIndef = 843,
    /// `ScrCmd_BufferItemNamePlural` (opcode 844).
    BufferItemNamePlural = 844,
    /// `ScrCmd_BufferPartyMonSpeciesNameIndef` (opcode 845).
    BufferPartyMonSpeciesNameIndef = 845,
    /// `ScrCmd_BufferSpeciesNameIndef` (opcode 846).
    BufferSpeciesNameIndef = 846,
    /// `ScrCmd_BufferDPPtFriendStarterSpeciesNameIndef` (opcode 847).
    BufferDPPtFriendStarterSpeciesNameIndef = 847,
    /// `ScrCmd_BufferFashionNameIndef` (opcode 848).
    BufferFashionNameIndef = 848,
    /// `ScrCmd_BufferTrainerClassNameIndef` (opcode 849).
    BufferTrainerClassNameIndef = 849,
    /// `ScrCmd_BufferSealNamePlural` (opcode 850).
    BufferSealNamePlural = 850,
    /// `ScrCmd_Capitalize` (opcode 851).
    Capitalize = 851,
    /// `ScrCmd_BufferDeptStoreFloorNo` (opcode 852).
    BufferDeptStoreFloorNo = 852,
}

/// The table in opcode order, for `u16 -> Opcode` lookup without `unsafe`.
static ALL: [Opcode; OPCODE_COUNT] = [
    Opcode::Nop,
    Opcode::Dummy,
    Opcode::End,
    Opcode::Wait,
    Opcode::LoadByte,
    Opcode::LoadWord,
    Opcode::LoadByteFromAddr,
    Opcode::WriteByteToAddr,
    Opcode::SetPtrByte,
    Opcode::CopyLocal,
    Opcode::CopyByte,
    Opcode::CompareLocalToLocal,
    Opcode::CompareLocalToValue,
    Opcode::CompareLocalToAddr,
    Opcode::CompareAddrToLocal,
    Opcode::CompareAddrToValue,
    Opcode::CompareAddrToAddr,
    Opcode::CompareVarToValue,
    Opcode::CompareVarToVar,
    Opcode::RunScript,
    Opcode::CallStd,
    Opcode::RestartCurrentScript,
    Opcode::GoTo,
    Opcode::ObjectGoTo,
    Opcode::BGGoTo,
    Opcode::DirectionGoTo,
    Opcode::Call,
    Opcode::Return,
    Opcode::GoToIf,
    Opcode::CallIf,
    Opcode::SetFlag,
    Opcode::ClearFlag,
    Opcode::CheckFlag,
    Opcode::SetFlagVar,
    Opcode::ClearFlagVar,
    Opcode::CheckFlagVar,
    Opcode::SetTrainerFlag,
    Opcode::ClearTrainerFlag,
    Opcode::CheckTrainerFlag,
    Opcode::AddVar,
    Opcode::SubVar,
    Opcode::SetVar,
    Opcode::CopyVar,
    Opcode::SetOrCopyVar,
    Opcode::NonNPCMsg,
    Opcode::NPCMsg,
    Opcode::NonNPCMsgVar,
    Opcode::NPCMsgVar,
    Opcode::Cmd048,
    Opcode::WaitABPress,
    Opcode::WaitButton,
    Opcode::WaitButtonOrDpad,
    Opcode::OpenMsg,
    Opcode::CloseMsg,
    Opcode::HoldMsg,
    Opcode::DirectionSignpost,
    Opcode::SetSignpostMap,
    Opcode::SetSignpostAction,
    Opcode::WaitSignpostAction,
    Opcode::TrainerTips,
    Opcode::WaitSignpost,
    Opcode::Cmd061,
    Opcode::Cmd062,
    Opcode::YesNo,
    Opcode::Cmd064,
    Opcode::Cmd065,
    Opcode::Cmd066,
    Opcode::Cmd067,
    Opcode::Cmd068,
    Opcode::Cmd069,
    Opcode::Cmd070,
    Opcode::Cmd071,
    Opcode::Cmd072,
    Opcode::PlaySE,
    Opcode::StopSE,
    Opcode::WaitSE,
    Opcode::PlayCry,
    Opcode::WaitCry,
    Opcode::PlayFanfare,
    Opcode::WaitFanfare,
    Opcode::PlayBGM,
    Opcode::StopBGM,
    Opcode::ResetBGM,
    Opcode::Cmd083,
    Opcode::FadeOutBGM,
    Opcode::FadeInBGM,
    Opcode::Cmd086,
    Opcode::TempBGM,
    Opcode::Cmd088,
    Opcode::ChatotHasCry,
    Opcode::ChatotStartRecording,
    Opcode::ChatotStopRecording,
    Opcode::ChatotSaveRecording,
    Opcode::Cmd093,
    Opcode::ApplyMovement,
    Opcode::WaitMovement,
    Opcode::LockAll,
    Opcode::ReleaseAll,
    Opcode::Lock,
    Opcode::Release,
    Opcode::ShowPerson,
    Opcode::HidePerson,
    Opcode::Cmd102,
    Opcode::Cmd103,
    Opcode::FacePlayer,
    Opcode::GetPlayerCoords,
    Opcode::GetPersonCoords,
    Opcode::Cmd107,
    Opcode::Cmd108,
    Opcode::Cmd109,
    Opcode::AddMoney,
    Opcode::SubMoneyImmediate,
    Opcode::HasEnoughMoneyImmediate,
    Opcode::ShowMoneyBox,
    Opcode::HideMoneyBox,
    Opcode::UpdateMoneyBox,
    Opcode::Cmd116,
    Opcode::Cmd117,
    Opcode::Cmd118,
    Opcode::GetCoinAmount,
    Opcode::GiveCoins,
    Opcode::TakeCoins,
    Opcode::GiveAthletePoints,
    Opcode::TakeAthletePoints,
    Opcode::CheckAthletePoints,
    Opcode::GiveItem,
    Opcode::TakeItem,
    Opcode::HasSpaceForItem,
    Opcode::HasItem,
    Opcode::ItemIsTMOrHM,
    Opcode::GetItemPocket,
    Opcode::SetStarterChoice,
    Opcode::GenderMsgBox,
    Opcode::GetSealQuantity,
    Opcode::GiveOrTakeSeal,
    Opcode::GiveRandomSeal,
    Opcode::Cmd136,
    Opcode::GiveMon,
    Opcode::GiveEgg,
    Opcode::SetMonMove,
    Opcode::MonHasMove,
    Opcode::GetPartySlotWithMove,
    Opcode::GetPhoneBookRematch,
    Opcode::NameRival,
    Opcode::GetFriendSprite,
    Opcode::RegisterPokegearCard,
    Opcode::RegisterGearNumber,
    Opcode::CheckRegisteredPhoneNumber,
    Opcode::Cmd148,
    Opcode::UnsetPhoneCallTrigger,
    Opcode::RestoreOverworld,
    Opcode::Cmd151,
    Opcode::Cmd152,
    Opcode::Cmd153,
    Opcode::Cmd154,
    Opcode::Cmd155,
    Opcode::Cmd156,
    Opcode::TownMap,
    Opcode::Cmd158,
    Opcode::Cmd159,
    Opcode::Cmd160,
    Opcode::Cmd161,
    Opcode::Cmd162,
    Opcode::HOFCredits,
    Opcode::Cmd164,
    Opcode::Cmd165,
    Opcode::Cmd166,
    Opcode::ChooseStarter,
    Opcode::GetTrainerPathToPlayer,
    Opcode::TrainerStepTowardsPlayer,
    Opcode::GetTrainerEyeType,
    Opcode::GetEyeTrainerNum,
    Opcode::NamePlayer,
    Opcode::NicknameInput,
    Opcode::FadeScreen,
    Opcode::WaitFade,
    Opcode::Warp,
    Opcode::RockClimb,
    Opcode::Surf,
    Opcode::Waterfall,
    Opcode::Cmd180,
    Opcode::FlashEffect,
    Opcode::Whirlpool,
    Opcode::Cmd183,
    Opcode::PlayerOnBikeCheck,
    Opcode::PlayerOnBikeSet,
    Opcode::SetBikeStateLock,
    Opcode::GetPlayerState,
    Opcode::SetAvatarBits,
    Opcode::UpdateAvatarState,
    Opcode::BufferPlayersName,
    Opcode::BufferRivalsName,
    Opcode::BufferFriendsName,
    Opcode::BufferMonSpeciesName,
    Opcode::BufferItemName,
    Opcode::BufferPocketName,
    Opcode::BufferTMHMMoveName,
    Opcode::BufferMoveName,
    Opcode::BufferInt,
    Opcode::BufferPartyMonNick,
    Opcode::BufferTrainerClassName,
    Opcode::BufferPlayerUnionAvatarClassName,
    Opcode::BufferSpeciesName,
    Opcode::BufferStarterSpeciesName,
    Opcode::BufferDPPtRivalStarterSpeciesName,
    Opcode::BufferDPPtFriendStarterSpeciesName,
    Opcode::GetStarterChoice,
    Opcode::BufferDecorationName,
    Opcode::Cmd208,
    Opcode::Cmd209,
    Opcode::BufferMapSecName,
    Opcode::Cmd211,
    Opcode::GetTrainerNum,
    Opcode::TrainerBattle,
    Opcode::TrainerMessage,
    Opcode::GetTrainerMsgParams,
    Opcode::GetRematchMsgParams,
    Opcode::TrainerIsDoubleBattle,
    Opcode::EncounterMusic,
    Opcode::WhiteOut,
    Opcode::CheckBattleWon,
    Opcode::StaticWildWonOrCaughtCheck,
    Opcode::PartyCheckForDouble,
    Opcode::Cmd223,
    Opcode::Cmd224,
    Opcode::GoToIfTrainerDefeated,
    Opcode::Cmd226,
    Opcode::Cmd227,
    Opcode::Cmd228,
    Opcode::Cmd229,
    Opcode::Cmd230,
    Opcode::Cmd231,
    Opcode::Cmd232,
    Opcode::Cmd233,
    Opcode::Cmd234,
    Opcode::Cmd235,
    Opcode::Cmd236,
    Opcode::Cmd237,
    Opcode::PartyHasPokerus,
    Opcode::MonGetGender,
    Opcode::SetDynamicWarp,
    Opcode::GetDynamicWarpFloorNo,
    Opcode::ElevatorCurFloorBox,
    Opcode::CountJohtoDexSeen,
    Opcode::CountJohtoDexOwned,
    Opcode::CountNationalDexSeen,
    Opcode::CountNationalDexOwned,
    Opcode::Cmd247,
    Opcode::GetDexEvalResult,
    Opcode::RocketTrapBattle,
    Opcode::Cmd250,
    Opcode::CatchingTutorial,
    Opcode::Cmd252,
    Opcode::GetSaveFileState,
    Opcode::SaveGameNormal,
    Opcode::Cmd255,
    Opcode::Cmd256,
    Opcode::Cmd257,
    Opcode::Cmd258,
    Opcode::Cmd259,
    Opcode::Cmd260,
    Opcode::Cmd261,
    Opcode::Cmd262,
    Opcode::Cmd263,
    Opcode::Cmd264,
    Opcode::Cmd265,
    Opcode::Cmd266,
    Opcode::Cmd267,
    Opcode::Cmd268,
    Opcode::Cmd269,
    Opcode::Cmd270,
    Opcode::Cmd271,
    Opcode::Cmd272,
    Opcode::Cmd273,
    Opcode::Cmd274,
    Opcode::MartBuy,
    Opcode::SpecialMartBuy,
    Opcode::DecorationMart,
    Opcode::SealMart,
    Opcode::OverworldWhiteOut,
    Opcode::SetSpawn,
    Opcode::GetPlayerGender,
    Opcode::HealParty,
    Opcode::Cmd283,
    Opcode::Cmd284,
    Opcode::Cmd285,
    Opcode::Cmd286,
    Opcode::BufferUnionRoomAvatarChoices,
    Opcode::UnionRoomAvatarIdxToTrainerClass,
    Opcode::Cmd289,
    Opcode::CheckPokedex,
    Opcode::GivePokedex,
    Opcode::CheckRunningShoes,
    Opcode::GiveRunningShoes,
    Opcode::CheckBadge,
    Opcode::GiveBadge,
    Opcode::CountBadges,
    Opcode::Cmd297,
    Opcode::Cmd298,
    Opcode::CheckEscortMode,
    Opcode::SetEscortMode,
    Opcode::ClearEscortMode,
    Opcode::CheckStepTakenFlag,
    Opcode::SetStepTakenFlag,
    Opcode::GetStepTakenFlag,
    Opcode::CheckGameClearFlag,
    Opcode::SetGameClearFlag,
    Opcode::Cmd307,
    Opcode::Cmd308,
    Opcode::Cmd309,
    Opcode::Cmd310,
    Opcode::Cmd311,
    Opcode::BufferDaycareMonNicks,
    Opcode::GetDaycareState,
    Opcode::EcruteakGymInit,
    Opcode::Cmd315,
    Opcode::Cmd316,
    Opcode::Cmd317,
    Opcode::CianwoodGymInit,
    Opcode::CianwoodGymTurnWinch,
    Opcode::VermilionGymInit,
    Opcode::VermilionGymLockAction,
    Opcode::VermilionGymCanCheck,
    Opcode::ResampleVermilionGymCans,
    Opcode::VioletGymInit,
    Opcode::VioletGymElevator,
    Opcode::AzaleaGymInit,
    Opcode::AzaleaGymSpinarak,
    Opcode::AzaleaGymSwitch,
    Opcode::BlackthornGymInit,
    Opcode::FuchsiaGymInit,
    Opcode::ViridianGymInit,
    Opcode::GetPartyCount,
    Opcode::Cmd333,
    Opcode::Cmd334,
    Opcode::Cmd335,
    Opcode::BufferBerryName,
    Opcode::BufferNatureName,
    Opcode::MovePerson,
    Opcode::MovePersonFacing,
    Opcode::SetObjectMovementType,
    Opcode::SetObjectFacing,
    Opcode::MoveWarp,
    Opcode::MoveBGEvent,
    Opcode::Cmd344,
    Opcode::AddWaitingIcon,
    Opcode::RemoveWaitingIcon,
    Opcode::Cmd347,
    Opcode::WaitButtonOrDelay,
    Opcode::PartySelectUI,
    Opcode::Cmd350,
    Opcode::GetPartySelection,
    Opcode::PokemonSummaryScreen,
    Opcode::GetMoveSelection,
    Opcode::GetPartyMonSpecies,
    Opcode::PartyMonIsMine,
    Opcode::PartyCountNotEgg,
    Opcode::CountAliveMons,
    Opcode::CountAliveMonsAndPC,
    Opcode::PartyCountEgg,
    Opcode::SubMoneyVar,
    Opcode::RetrieveDaycareMon,
    Opcode::GiveLoanMon,
    Opcode::CheckReturnLoanMon,
    Opcode::ReturnLoanMon,
    Opcode::ResetDaycareEgg,
    Opcode::GiveDaycareEgg,
    Opcode::BufferDaycareWithdrawCost,
    Opcode::HasEnoughMoneyVar,
    Opcode::EggHatchAnim,
    Opcode::Cmd370,
    Opcode::BufferDaycareMonGrowth,
    Opcode::GetTailDaycareMonSpeciesAndNick,
    Opcode::PutMonInDaycare,
    Opcode::Cmd374,
    Opcode::MakeObjectVisible,
    Opcode::Cmd376,
    Opcode::Cmd377,
    Opcode::ViewRankings,
    Opcode::Cmd379,
    Opcode::Random,
    Opcode::Cmd381,
    Opcode::MonGetFriendship,
    Opcode::MonAddFriendship,
    Opcode::MonSubtractFriendship,
    Opcode::BufferDaycareMonStats,
    Opcode::GetPlayerFacing,
    Opcode::GetDaycareCompatibility,
    Opcode::CheckDaycareEgg,
    Opcode::PlayerHasSpecies,
    Opcode::SizeRecordCompare,
    Opcode::SizeRecordUpdate,
    Opcode::BufferMonSize,
    Opcode::BufferRecordSize,
    Opcode::Cmd394,
    Opcode::Cmd395,
    Opcode::CountMonMoves,
    Opcode::MonForgetMove,
    Opcode::MonGetMove,
    Opcode::BufferPartyMonMoveName,
    Opcode::StrengthFlagAction,
    Opcode::FlashAction,
    Opcode::DefogAction,
    Opcode::Cmd403,
    Opcode::Cmd404,
    Opcode::Cmd405,
    Opcode::Cmd406,
    Opcode::Cmd407,
    Opcode::Cmd408,
    Opcode::Cmd409,
    Opcode::Cmd410,
    Opcode::Cmd411,
    Opcode::Cmd412,
    Opcode::Cmd413,
    Opcode::Cmd414,
    Opcode::Cmd415,
    Opcode::Cmd416,
    Opcode::Cmd417,
    Opcode::Cmd418,
    Opcode::Cmd419,
    Opcode::Cmd420,
    Opcode::Cmd421,
    Opcode::Cmd422,
    Opcode::CheckJohtoDexComplete,
    Opcode::CheckNationalDexComplete,
    Opcode::ShowCertificate,
    Opcode::KenyaCheck,
    Opcode::Cmd427,
    Opcode::MonGiveMail,
    Opcode::CountFossils,
    Opcode::SetPhoneCall,
    Opcode::RunPhoneCall,
    Opcode::GetFossilPokemon,
    Opcode::GetFossilMinimumAmount,
    Opcode::PartyCountMonsAtOrBelowLevel,
    Opcode::SurvivePoisoning,
    Opcode::Cmd436,
    Opcode::DebugWatch,
    Opcode::GetStdMsgNaix,
    Opcode::NonNPCMsgExtern,
    Opcode::MsgBoxExtern,
    Opcode::Cmd441,
    Opcode::Cmd442,
    Opcode::Cmd443,
    Opcode::Cmd444,
    Opcode::Cmd445,
    Opcode::Cmd446,
    Opcode::SafariZoneAction,
    Opcode::Cmd448,
    Opcode::Cmd449,
    Opcode::Cmd450,
    Opcode::Cmd451,
    Opcode::Cmd452,
    Opcode::Cmd453,
    Opcode::Cmd454,
    Opcode::Cmd455,
    Opcode::Cmd456,
    Opcode::MonGetNature,
    Opcode::GetPartySlotWithNature,
    Opcode::Cmd459,
    Opcode::LoadPhoneDat,
    Opcode::GetPhoneContactMsgIds,
    Opcode::Cmd462,
    Opcode::EnableMassOutbreaks,
    Opcode::CreateRoamer,
    Opcode::Cmd465,
    Opcode::Cmd466,
    Opcode::MoveRelearnerInit,
    Opcode::MoveTutorInit,
    Opcode::MoveRelearnerGetResult,
    Opcode::LoadNPCTrade,
    Opcode::GetOfferedSpecies,
    Opcode::NPCTradeGetReqSpecies,
    Opcode::NPCTradeExec,
    Opcode::NPCTradeEnd,
    Opcode::Cmd475,
    Opcode::EnablePokedexFormDetection,
    Opcode::NatDexFlagAction,
    Opcode::MonGetRibbonCount,
    Opcode::GetPartyRibbonCount,
    Opcode::MonHasRibbon,
    Opcode::GiveRibbon,
    Opcode::BufferRibbonName,
    Opcode::GetEVTotal,
    Opcode::GetWeekday,
    Opcode::StartBattleRegulationMenuTask,
    Opcode::Dummy486,
    Opcode::PokeCenAnim,
    Opcode::ElevatorAnim,
    Opcode::MysteryGift,
    Opcode::NopVar490,
    Opcode::Cmd491,
    Opcode::Cmd492,
    Opcode::PromptEasyChat,
    Opcode::Cmd494,
    Opcode::GetGameVersion,
    Opcode::GetPartyLead,
    Opcode::GetMonTypes,
    Opcode::PrimoPasswordCheck1,
    Opcode::PrimoPasswordCheck2,
    Opcode::Cmd500,
    Opcode::Cmd501,
    Opcode::Cmd502,
    Opcode::LotoIDGet,
    Opcode::LotoIDSearch,
    Opcode::LotoIDSet,
    Opcode::BufferBoxMonNick,
    Opcode::CountPCEmptySpace,
    Opcode::PalParkAction,
    Opcode::Cmd509,
    Opcode::Cmd510,
    Opcode::PalParkScoreGet,
    Opcode::PlayerMovementSavingSet,
    Opcode::PlayerMovementSavingClear,
    Opcode::HallOfFameAnim,
    Opcode::AddSpecialGameStat,
    Opcode::BufferFashionName,
    Opcode::Cmd517,
    Opcode::Cmd518,
    Opcode::Cmd519,
    Opcode::Cmd520,
    Opcode::Cmd521,
    Opcode::Cmd522,
    Opcode::Cmd523,
    Opcode::Cmd524,
    Opcode::Cmd525,
    Opcode::Cmd526,
    Opcode::Cmd527,
    Opcode::Cmd528,
    Opcode::GetPartyLeadAlive,
    Opcode::Cmd530,
    Opcode::BufferBackgroundName,
    Opcode::CheckCoinsImmediate,
    Opcode::CheckGiveCoins,
    Opcode::Cmd534,
    Opcode::MonGetLevel,
    Opcode::Cmd536,
    Opcode::Cmd537,
    Opcode::Cmd538,
    Opcode::Cmd539,
    Opcode::Cmd540,
    Opcode::BufferIntEx,
    Opcode::MonGetContestValue,
    Opcode::Cmd543,
    Opcode::Cmd544,
    Opcode::Cmd545,
    Opcode::Cmd546,
    Opcode::Cmd547,
    Opcode::Cmd548,
    Opcode::Cmd549,
    Opcode::Cmd550,
    Opcode::Cmd551,
    Opcode::Cmd552,
    Opcode::Cmd553,
    Opcode::Cmd554,
    Opcode::Cmd555,
    Opcode::Cmd556,
    Opcode::CheckBattlePoints,
    Opcode::UnionRoomAvatarIdxToSprite,
    Opcode::Cmd559,
    Opcode::Cmd560,
    Opcode::ScreenShake,
    Opcode::MultiBattle,
    Opcode::Cmd563,
    Opcode::Cmd564,
    Opcode::Cmd565,
    Opcode::Cmd566,
    Opcode::GetDPPlPrizeItemIDAndCost,
    Opcode::Cmd568,
    Opcode::Cmd569,
    Opcode::CheckCoinsVar,
    Opcode::Cmd571,
    Opcode::GetUniqueSealsQuantity,
    Opcode::Cmd573,
    Opcode::Cmd574,
    Opcode::Cmd575,
    Opcode::Cmd576,
    Opcode::Cmd577,
    Opcode::Cmd578,
    Opcode::Cmd579,
    Opcode::BufferSealName,
    Opcode::LockLastTalked,
    Opcode::Cmd582,
    Opcode::Cmd583,
    Opcode::PartyLegalCheck,
    Opcode::Cmd585,
    Opcode::Cmd586,
    Opcode::Cmd587,
    Opcode::LatiCaughtCheck,
    Opcode::WildBattle,
    Opcode::GetTrcardStars,
    Opcode::Cmd591,
    Opcode::Cmd592,
    Opcode::ShowSaveStats,
    Opcode::HideSaveStats,
    Opcode::Cmd595,
    Opcode::Cmd596,
    Opcode::Cmd597,
    Opcode::Cmd598,
    Opcode::Cmd599,
    Opcode::Cmd600,
    Opcode::FollowMonFacePlayer,
    Opcode::ToggleFollowingPokemonMovement,
    Opcode::WaitFollowingPokemonMovement,
    Opcode::FollowingPokemonMovement,
    Opcode::Cmd605,
    Opcode::Cmd606,
    Opcode::Cmd607,
    Opcode::Cmd608,
    Opcode::Cmd609,
    Opcode::Cmd610,
    Opcode::Pokeathlon,
    Opcode::GetNPCTradeUnusedFlag,
    Opcode::GetPhoneContactRandomGiftBerry,
    Opcode::GetPhoneContactGiftItem,
    Opcode::CameronPhoto,
    Opcode::CountSavedPhotos,
    Opcode::OpenPhotoAlbum,
    Opcode::PhotoAlbumIsFull,
    Opcode::RocketCostumeFlagCheck,
    Opcode::RocketCostumeFlagAction,
    Opcode::PlaceStarterBallsInElmsLab,
    Opcode::Cmd622,
    Opcode::AnimApricornTree,
    Opcode::ApricornTreeGetApricorn,
    Opcode::GiveApricornFromTree,
    Opcode::BufferApricornName,
    Opcode::Cmd627,
    Opcode::Cmd628,
    Opcode::Cmd629,
    Opcode::Cmd630,
    Opcode::Cmd631,
    Opcode::CountPartyMonsOfSpecies,
    Opcode::Cmd633,
    Opcode::Cmd634,
    Opcode::Cmd635,
    Opcode::Cmd636,
    Opcode::Cmd637,
    Opcode::Cmd638,
    Opcode::Cmd639,
    Opcode::Cmd640,
    Opcode::SaveWipeExtraChunks,
    Opcode::Cmd642,
    Opcode::Cmd643,
    Opcode::Cmd644,
    Opcode::Cmd645,
    Opcode::Cmd646,
    Opcode::GetPartySlotWithSpecies,
    Opcode::Cmd648,
    Opcode::ScratchOffCard,
    Opcode::ScratchOffCardEnd,
    Opcode::GetScratchOffPrize,
    Opcode::Cmd652,
    Opcode::MoveTutorChooseMove,
    Opcode::TutorMoveTeachInSlot,
    Opcode::TutorMoveGetPrice,
    Opcode::Cmd656,
    Opcode::StatJudge,
    Opcode::BufferStatName,
    Opcode::SetMonForm,
    Opcode::BufferTrainerName,
    Opcode::Cmd661,
    Opcode::Cmd662,
    Opcode::Cmd663,
    Opcode::Cmd664,
    Opcode::Cmd665,
    Opcode::Cmd666,
    Opcode::Cmd667,
    Opcode::BufferTypeName,
    Opcode::GetItemQuantity,
    Opcode::GetHiddenPowerType,
    Opcode::SetFavoriteMon,
    Opcode::GetFavoriteMon,
    Opcode::GetOwnedRotomForms,
    Opcode::CountTranformedRotomsInParty,
    Opcode::UpdateRotomForm,
    Opcode::GetPartyMonForm,
    Opcode::Cmd677,
    Opcode::Cmd678,
    Opcode::Cmd679,
    Opcode::AddSpecialGameStat2,
    Opcode::Cmd681,
    Opcode::Cmd682,
    Opcode::GetStaticEncounterOutcome,
    Opcode::Cmd684,
    Opcode::GetPlayerXYZ,
    Opcode::Cmd686,
    Opcode::Cmd687,
    Opcode::GetPartySlotWithFatefulEncounter,
    Opcode::CommSanitizeParty,
    Opcode::DaycareSanitizeMon,
    Opcode::Cmd691,
    Opcode::BufferBattleHallStreak,
    Opcode::BattleHallCountUsedSpecies,
    Opcode::BattleHallGetTotalStreak,
    Opcode::Cmd695,
    Opcode::Cmd696,
    Opcode::Cmd697,
    Opcode::FollowerPokeIsEventTrigger,
    Opcode::Cmd699,
    Opcode::Cmd700,
    Opcode::MonHasItem,
    Opcode::BattleTowerSetUpMultiBattle,
    Opcode::SetPlayerVolume,
    Opcode::Cmd704,
    Opcode::Cmd705,
    Opcode::Cmd706,
    Opcode::CheckMonSeen,
    Opcode::Cmd708,
    Opcode::Cmd709,
    Opcode::Cmd710,
    Opcode::FollowMonInteract,
    Opcode::Cmd712,
    Opcode::AlphPuzzle,
    Opcode::OpenAlphHiddenRoom,
    Opcode::UpdateDaycareMonObjects,
    Opcode::Cmd716,
    Opcode::Cmd717,
    Opcode::Cmd718,
    Opcode::Cmd719,
    Opcode::Cmd720,
    Opcode::Cmd721,
    Opcode::Cmd722,
    Opcode::Cmd723,
    Opcode::Cmd724,
    Opcode::Cmd725,
    Opcode::ProcessSoundplate,
    Opcode::GetFollowPokePartyIndex,
    Opcode::Cmd728,
    Opcode::Cmd729,
    Opcode::Cmd730,
    Opcode::Cmd731,
    Opcode::Cmd732,
    Opcode::Cmd733,
    Opcode::Cmd734,
    Opcode::Cmd735,
    Opcode::ClearKurtApricorn,
    Opcode::Cmd737,
    Opcode::GetTotalApricornCount,
    Opcode::Cmd739,
    Opcode::Cmd740,
    Opcode::Cmd741,
    Opcode::Cmd742,
    Opcode::Cmd743,
    Opcode::CreatePokeathlonFriendshipRoomStatues,
    Opcode::BufferPokeathlonCourseName,
    Opcode::TouchscreenMenuHide,
    Opcode::TouchscreenMenuShow,
    Opcode::GetMenuChoice,
    Opcode::MenuInitStdGmm,
    Opcode::MenuInit,
    Opcode::MenuItemAdd,
    Opcode::MenuExec,
    Opcode::RockSmashItemCheck,
    Opcode::TryHeadbuttEncounter,
    Opcode::LegendCutsceneClearBellAnimBegin,
    Opcode::LegendCutsceneClearBellAnimEnd,
    Opcode::LegendCutsceneClearBellRiseFromBag,
    Opcode::LegendCutsceneClearBellShimmer,
    Opcode::LegendCutsceneLugiaEyeGlimmerEffect,
    Opcode::Cmd760,
    Opcode::LegendCutsceneMoveCameraTo,
    Opcode::LegendCutscenePanCameraTo,
    Opcode::LegendCutsceneWaitCameraPan,
    Opcode::LegendCutsceneBirdFinalApproach,
    Opcode::LegendCutsceneWavesOrLeavesEffectBegin,
    Opcode::LegendCutsceneWavesOrLeavesEffectEnd,
    Opcode::LegendCutsceneLugiaArrivesEffectBegin,
    Opcode::LegendCutsceneLugiaArrivesEffectEnd,
    Opcode::LegendCutsceneLugiaArrivesEffectCameraPan,
    Opcode::CheckSeenAllLetterUnown,
    Opcode::Cmd771,
    Opcode::Cmd772,
    Opcode::Cinematic,
    Opcode::ShowLegendaryWing,
    Opcode::Cmd775,
    Opcode::GiveTogepiEgg,
    Opcode::Cmd777,
    Opcode::GiveSpikyEarPichu,
    Opcode::RadioMusicIsPlaying,
    Opcode::CasinoGame,
    Opcode::KenyaCheckPartyOrMailbox,
    Opcode::MartSell,
    Opcode::SetFollowMonInhibitState,
    Opcode::ScriptOverlayCmd,
    Opcode::BugContestAction,
    Opcode::BufferBugContestWinner,
    Opcode::JudgeBugContest,
    Opcode::BufferBugContestMonNick,
    Opcode::BugContestGetTimeLeft,
    Opcode::IsBugContestantRegistered,
    Opcode::CheckSafariZoneChallengeCompleted,
    Opcode::UpdateSafariZoneIGT,
    Opcode::BankTransaction,
    Opcode::CheckBankBalance,
    Opcode::Cmd795,
    Opcode::Cmd796,
    Opcode::Cmd797,
    Opcode::BufferRulesetName,
    Opcode::Cmd799,
    Opcode::Cmd800,
    Opcode::Cmd801,
    Opcode::Cmd802,
    Opcode::Cmd803,
    Opcode::Cmd804,
    Opcode::Cmd805,
    Opcode::Cmd806,
    Opcode::SetTrainerHouseSprite,
    Opcode::Cmd808,
    Opcode::ShowTrainerHouseIntroMessage,
    Opcode::Cmd810,
    Opcode::Cmd811,
    Opcode::Cmd812,
    Opcode::MomGiftCheck,
    Opcode::Cmd814,
    Opcode::Cmd815,
    Opcode::UnownCircle,
    Opcode::Cmd817,
    Opcode::MystriStageGymmickInit,
    Opcode::Cmd819,
    Opcode::Cmd820,
    Opcode::GetBuenasPassword,
    Opcode::Cmd822,
    Opcode::Cmd823,
    Opcode::Cmd824,
    Opcode::GetShinyLeafCount,
    Opcode::TryGiveShinyLeafCrown,
    Opcode::GetPartyMonForm2,
    Opcode::MonAddContestValue,
    Opcode::Cmd829,
    Opcode::Cmd830,
    Opcode::Cmd831,
    Opcode::Cmd832,
    Opcode::Cmd833,
    Opcode::Cmd834,
    Opcode::Cmd835,
    Opcode::CheckKyogreGroudonInParty,
    Opcode::Cmd837,
    Opcode::BankOrWalletIsFull,
    Opcode::SysSetSleepFlag,
    Opcode::Cmd840,
    Opcode::Cmd841,
    Opcode::Cmd842,
    Opcode::BufferItemNameIndef,
    Opcode::BufferItemNamePlural,
    Opcode::BufferPartyMonSpeciesNameIndef,
    Opcode::BufferSpeciesNameIndef,
    Opcode::BufferDPPtFriendStarterSpeciesNameIndef,
    Opcode::BufferFashionNameIndef,
    Opcode::BufferTrainerClassNameIndef,
    Opcode::BufferSealNamePlural,
    Opcode::Capitalize,
    Opcode::BufferDeptStoreFloorNo,
];

/// pret's identifier for each entry (the `ScrCmd_` prefix kept).
static NAMES: [&str; OPCODE_COUNT] = [
    "ScrCmd_Nop",
    "ScrCmd_Dummy",
    "ScrCmd_End",
    "ScrCmd_Wait",
    "ScrCmd_LoadByte",
    "ScrCmd_LoadWord",
    "ScrCmd_LoadByteFromAddr",
    "ScrCmd_WriteByteToAddr",
    "ScrCmd_SetPtrByte",
    "ScrCmd_CopyLocal",
    "ScrCmd_CopyByte",
    "ScrCmd_CompareLocalToLocal",
    "ScrCmd_CompareLocalToValue",
    "ScrCmd_CompareLocalToAddr",
    "ScrCmd_CompareAddrToLocal",
    "ScrCmd_CompareAddrToValue",
    "ScrCmd_CompareAddrToAddr",
    "ScrCmd_CompareVarToValue",
    "ScrCmd_CompareVarToVar",
    "ScrCmd_RunScript",
    "ScrCmd_CallStd",
    "ScrCmd_RestartCurrentScript",
    "ScrCmd_GoTo",
    "ScrCmd_ObjectGoTo",
    "ScrCmd_BGGoTo",
    "ScrCmd_DirectionGoTo",
    "ScrCmd_Call",
    "ScrCmd_Return",
    "ScrCmd_GoToIf",
    "ScrCmd_CallIf",
    "ScrCmd_SetFlag",
    "ScrCmd_ClearFlag",
    "ScrCmd_CheckFlag",
    "ScrCmd_SetFlagVar",
    "ScrCmd_ClearFlagVar",
    "ScrCmd_CheckFlagVar",
    "ScrCmd_SetTrainerFlag",
    "ScrCmd_ClearTrainerFlag",
    "ScrCmd_CheckTrainerFlag",
    "ScrCmd_AddVar",
    "ScrCmd_SubVar",
    "ScrCmd_SetVar",
    "ScrCmd_CopyVar",
    "ScrCmd_SetOrCopyVar",
    "ScrCmd_NonNPCMsg",
    "ScrCmd_NPCMsg",
    "ScrCmd_NonNPCMsgVar",
    "ScrCmd_NPCMsgVar",
    "ScrCmd_048",
    "ScrCmd_WaitABPress",
    "ScrCmd_WaitButton",
    "ScrCmd_WaitButtonOrDpad",
    "ScrCmd_OpenMsg",
    "ScrCmd_CloseMsg",
    "ScrCmd_HoldMsg",
    "ScrCmd_DirectionSignpost",
    "ScrCmd_SetSignpostMap",
    "ScrCmd_SetSignpostAction",
    "ScrCmd_WaitSignpostAction",
    "ScrCmd_TrainerTips",
    "ScrCmd_WaitSignpost",
    "ScrCmd_061",
    "ScrCmd_062",
    "ScrCmd_YesNo",
    "ScrCmd_064",
    "ScrCmd_065",
    "ScrCmd_066",
    "ScrCmd_067",
    "ScrCmd_068",
    "ScrCmd_069",
    "ScrCmd_070",
    "ScrCmd_071",
    "ScrCmd_072",
    "ScrCmd_PlaySE",
    "ScrCmd_StopSE",
    "ScrCmd_WaitSE",
    "ScrCmd_PlayCry",
    "ScrCmd_WaitCry",
    "ScrCmd_PlayFanfare",
    "ScrCmd_WaitFanfare",
    "ScrCmd_PlayBGM",
    "ScrCmd_StopBGM",
    "ScrCmd_ResetBGM",
    "ScrCmd_083",
    "ScrCmd_FadeOutBGM",
    "ScrCmd_FadeInBGM",
    "ScrCmd_086",
    "ScrCmd_TempBGM",
    "ScrCmd_088",
    "ScrCmd_ChatotHasCry",
    "ScrCmd_ChatotStartRecording",
    "ScrCmd_ChatotStopRecording",
    "ScrCmd_ChatotSaveRecording",
    "ScrCmd_093",
    "ScrCmd_ApplyMovement",
    "ScrCmd_WaitMovement",
    "ScrCmd_LockAll",
    "ScrCmd_ReleaseAll",
    "ScrCmd_Lock",
    "ScrCmd_Release",
    "ScrCmd_ShowPerson",
    "ScrCmd_HidePerson",
    "ScrCmd_102",
    "ScrCmd_103",
    "ScrCmd_FacePlayer",
    "ScrCmd_GetPlayerCoords",
    "ScrCmd_GetPersonCoords",
    "ScrCmd_107",
    "ScrCmd_108",
    "ScrCmd_109",
    "ScrCmd_AddMoney",
    "ScrCmd_SubMoneyImmediate",
    "ScrCmd_HasEnoughMoneyImmediate",
    "ScrCmd_ShowMoneyBox",
    "ScrCmd_HideMoneyBox",
    "ScrCmd_UpdateMoneyBox",
    "ScrCmd_116",
    "ScrCmd_117",
    "ScrCmd_118",
    "ScrCmd_GetCoinAmount",
    "ScrCmd_GiveCoins",
    "ScrCmd_TakeCoins",
    "ScrCmd_GiveAthletePoints",
    "ScrCmd_TakeAthletePoints",
    "ScrCmd_CheckAthletePoints",
    "ScrCmd_GiveItem",
    "ScrCmd_TakeItem",
    "ScrCmd_HasSpaceForItem",
    "ScrCmd_HasItem",
    "ScrCmd_ItemIsTMOrHM",
    "ScrCmd_GetItemPocket",
    "ScrCmd_SetStarterChoice",
    "ScrCmd_GenderMsgBox",
    "ScrCmd_GetSealQuantity",
    "ScrCmd_GiveOrTakeSeal",
    "ScrCmd_GiveRandomSeal",
    "ScrCmd_136",
    "ScrCmd_GiveMon",
    "ScrCmd_GiveEgg",
    "ScrCmd_SetMonMove",
    "ScrCmd_MonHasMove",
    "ScrCmd_GetPartySlotWithMove",
    "ScrCmd_GetPhoneBookRematch",
    "ScrCmd_NameRival",
    "ScrCmd_GetFriendSprite",
    "ScrCmd_RegisterPokegearCard",
    "ScrCmd_RegisterGearNumber",
    "ScrCmd_CheckRegisteredPhoneNumber",
    "ScrCmd_148",
    "UnsetPhoneCallTrigger",
    "ScrCmd_RestoreOverworld",
    "ScrCmd_151",
    "ScrCmd_152",
    "ScrCmd_153",
    "ScrCmd_154",
    "ScrCmd_155",
    "ScrCmd_156",
    "ScrCmd_TownMap",
    "ScrCmd_158",
    "ScrCmd_159",
    "ScrCmd_160",
    "ScrCmd_161",
    "ScrCmd_162",
    "ScrCmd_HOFCredits",
    "ScrCmd_164",
    "ScrCmd_165",
    "ScrCmd_166",
    "ScrCmd_ChooseStarter",
    "ScrCmd_GetTrainerPathToPlayer",
    "ScrCmd_TrainerStepTowardsPlayer",
    "ScrCmd_GetTrainerEyeType",
    "ScrCmd_GetEyeTrainerNum",
    "ScrCmd_NamePlayer",
    "ScrCmd_NicknameInput",
    "ScrCmd_FadeScreen",
    "ScrCmd_WaitFade",
    "ScrCmd_Warp",
    "ScrCmd_RockClimb",
    "ScrCmd_Surf",
    "ScrCmd_Waterfall",
    "ScrCmd_180",
    "ScrCmd_FlashEffect",
    "ScrCmd_Whirlpool",
    "ScrCmd_183",
    "ScrCmd_PlayerOnBikeCheck",
    "ScrCmd_PlayerOnBikeSet",
    "ScrCmd_SetBikeStateLock",
    "ScrCmd_GetPlayerState",
    "ScrCmd_SetAvatarBits",
    "ScrCmd_UpdateAvatarState",
    "ScrCmd_BufferPlayersName",
    "ScrCmd_BufferRivalsName",
    "ScrCmd_BufferFriendsName",
    "ScrCmd_BufferMonSpeciesName",
    "ScrCmd_BufferItemName",
    "ScrCmd_BufferPocketName",
    "ScrCmd_BufferTMHMMoveName",
    "ScrCmd_BufferMoveName",
    "ScrCmd_BufferInt",
    "ScrCmd_BufferPartyMonNick",
    "ScrCmd_BufferTrainerClassName",
    "ScrCmd_BufferPlayerUnionAvatarClassName",
    "ScrCmd_BufferSpeciesName",
    "ScrCmd_BufferStarterSpeciesName",
    "ScrCmd_BufferDPPtRivalStarterSpeciesName",
    "ScrCmd_BufferDPPtFriendStarterSpeciesName",
    "ScrCmd_GetStarterChoice",
    "ScrCmd_BufferDecorationName",
    "ScrCmd_208",
    "ScrCmd_209",
    "ScrCmd_BufferMapSecName",
    "ScrCmd_211",
    "ScrCmd_GetTrainerNum",
    "ScrCmd_TrainerBattle",
    "ScrCmd_TrainerMessage",
    "ScrCmd_GetTrainerMsgParams",
    "ScrCmd_GetRematchMsgParams",
    "ScrCmd_TrainerIsDoubleBattle",
    "ScrCmd_EncounterMusic",
    "ScrCmd_WhiteOut",
    "ScrCmd_CheckBattleWon",
    "ScrCmd_StaticWildWonOrCaughtCheck",
    "ScrCmd_PartyCheckForDouble",
    "ScrCmd_223",
    "ScrCmd_224",
    "ScrCmd_GoToIfTrainerDefeated",
    "ScrCmd_226",
    "ScrCmd_227",
    "ScrCmd_228",
    "ScrCmd_229",
    "ScrCmd_230",
    "ScrCmd_231",
    "ScrCmd_232",
    "ScrCmd_233",
    "ScrCmd_234",
    "ScrCmd_235",
    "ScrCmd_236",
    "ScrCmd_237",
    "ScrCmd_PartyHasPokerus",
    "ScrCmd_MonGetGender",
    "ScrCmd_SetDynamicWarp",
    "ScrCmd_GetDynamicWarpFloorNo",
    "ScrCmd_ElevatorCurFloorBox",
    "ScrCmd_CountJohtoDexSeen",
    "ScrCmd_CountJohtoDexOwned",
    "ScrCmd_CountNationalDexSeen",
    "ScrCmd_CountNationalDexOwned",
    "ScrCmd_247",
    "ScrCmd_GetDexEvalResult",
    "ScrCmd_RocketTrapBattle",
    "ScrCmd_250",
    "ScrCmd_CatchingTutorial",
    "ScrCmd_252",
    "ScrCmd_GetSaveFileState",
    "ScrCmd_SaveGameNormal",
    "ScrCmd_255",
    "ScrCmd_256",
    "ScrCmd_257",
    "ScrCmd_258",
    "ScrCmd_259",
    "ScrCmd_260",
    "ScrCmd_261",
    "ScrCmd_262",
    "ScrCmd_263",
    "ScrCmd_264",
    "ScrCmd_265",
    "ScrCmd_266",
    "ScrCmd_267",
    "ScrCmd_268",
    "ScrCmd_269",
    "ScrCmd_270",
    "ScrCmd_271",
    "ScrCmd_272",
    "ScrCmd_273",
    "ScrCmd_274",
    "ScrCmd_MartBuy",
    "ScrCmd_SpecialMartBuy",
    "ScrCmd_DecorationMart",
    "ScrCmd_SealMart",
    "ScrCmd_OverworldWhiteOut",
    "ScrCmd_SetSpawn",
    "ScrCmd_GetPlayerGender",
    "ScrCmd_HealParty",
    "ScrCmd_283",
    "ScrCmd_284",
    "ScrCmd_285",
    "ScrCmd_286",
    "ScrCmd_BufferUnionRoomAvatarChoices",
    "ScrCmd_UnionRoomAvatarIdxToTrainerClass",
    "ScrCmd_289",
    "ScrCmd_CheckPokedex",
    "ScrCmd_GivePokedex",
    "ScrCmd_CheckRunningShoes",
    "ScrCmd_GiveRunningShoes",
    "ScrCmd_CheckBadge",
    "ScrCmd_GiveBadge",
    "ScrCmd_CountBadges",
    "ScrCmd_297",
    "ScrCmd_298",
    "ScrCmd_CheckEscortMode",
    "ScrCmd_SetEscortMode",
    "ScrCmd_ClearEscortMode",
    "ScrCmd_CheckStepTakenFlag",
    "ScrCmd_SetStepTakenFlag",
    "ScrCmd_GetStepTakenFlag",
    "ScrCmd_CheckGameClearFlag",
    "ScrCmd_SetGameClearFlag",
    "ScrCmd_307",
    "ScrCmd_308",
    "ScrCmd_309",
    "ScrCmd_310",
    "ScrCmd_311",
    "ScrCmd_BufferDaycareMonNicks",
    "ScrCmd_GetDaycareState",
    "ScrCmd_EcruteakGymInit",
    "ScrCmd_315",
    "ScrCmd_316",
    "ScrCmd_317",
    "ScrCmd_CianwoodGymInit",
    "ScrCmd_CianwoodGymTurnWinch",
    "ScrCmd_VermilionGymInit",
    "ScrCmd_VermilionGymLockAction",
    "ScrCmd_VermilionGymCanCheck",
    "ScrCmd_ResampleVermilionGymCans",
    "ScrCmd_VioletGymInit",
    "ScrCmd_VioletGymElevator",
    "ScrCmd_AzaleaGymInit",
    "ScrCmd_AzaleaGymSpinarak",
    "ScrCmd_AzaleaGymSwitch",
    "ScrCmd_BlackthornGymInit",
    "ScrCmd_FuchsiaGymInit",
    "ScrCmd_ViridianGymInit",
    "ScrCmd_GetPartyCount",
    "ScrCmd_333",
    "ScrCmd_334",
    "ScrCmd_335",
    "ScrCmd_BufferBerryName",
    "ScrCmd_BufferNatureName",
    "ScrCmd_MovePerson",
    "ScrCmd_MovePersonFacing",
    "ScrCmd_SetObjectMovementType",
    "ScrCmd_SetObjectFacing",
    "ScrCmd_MoveWarp",
    "ScrCmd_MoveBGEvent",
    "ScrCmd_344",
    "ScrCmd_AddWaitingIcon",
    "ScrCmd_RemoveWaitingIcon",
    "ScrCmd_347",
    "ScrCmd_WaitButtonOrDelay",
    "ScrCmd_PartySelectUI",
    "ScrCmd_350",
    "ScrCmd_GetPartySelection",
    "ScrCmd_PokemonSummaryScreen",
    "ScrCmd_GetMoveSelection",
    "ScrCmd_GetPartyMonSpecies",
    "ScrCmd_PartyMonIsMine",
    "ScrCmd_PartyCountNotEgg",
    "ScrCmd_CountAliveMons",
    "ScrCmd_CountAliveMonsAndPC",
    "ScrCmd_PartyCountEgg",
    "ScrCmd_SubMoneyVar",
    "ScrCmd_RetrieveDaycareMon",
    "ScrCmd_GiveLoanMon",
    "ScrCmd_CheckReturnLoanMon",
    "ScrCmd_ReturnLoanMon",
    "ScrCmd_ResetDaycareEgg",
    "ScrCmd_GiveDaycareEgg",
    "ScrCmd_BufferDaycareWithdrawCost",
    "ScrCmd_HasEnoughMoneyVar",
    "ScrCmd_EggHatchAnim",
    "ScrCmd_370",
    "ScrCmd_BufferDaycareMonGrowth",
    "ScrCmd_GetTailDaycareMonSpeciesAndNick",
    "ScrCmd_PutMonInDaycare",
    "ScrCmd_374",
    "ScrCmd_MakeObjectVisible",
    "ScrCmd_376",
    "ScrCmd_377",
    "ScrCmd_ViewRankings",
    "ScrCmd_379",
    "ScrCmd_Random",
    "ScrCmd_381",
    "ScrCmd_MonGetFriendship",
    "ScrCmd_MonAddFriendship",
    "ScrCmd_MonSubtractFriendship",
    "ScrCmd_BufferDaycareMonStats",
    "ScrCmd_GetPlayerFacing",
    "ScrCmd_GetDaycareCompatibility",
    "ScrCmd_CheckDaycareEgg",
    "ScrCmd_PlayerHasSpecies",
    "ScrCmd_SizeRecordCompare",
    "ScrCmd_SizeRecordUpdate",
    "ScrCmd_BufferMonSize",
    "ScrCmd_BufferRecordSize",
    "ScrCmd_394",
    "ScrCmd_395",
    "ScrCmd_CountMonMoves",
    "ScrCmd_MonForgetMove",
    "ScrCmd_MonGetMove",
    "ScrCmd_BufferPartyMonMoveName",
    "ScrCmd_StrengthFlagAction",
    "ScrCmd_FlashAction",
    "ScrCmd_DefogAction",
    "ScrCmd_403",
    "ScrCmd_404",
    "ScrCmd_405",
    "ScrCmd_406",
    "ScrCmd_407",
    "ScrCmd_408",
    "ScrCmd_409",
    "ScrCmd_410",
    "ScrCmd_411",
    "ScrCmd_412",
    "ScrCmd_413",
    "ScrCmd_414",
    "ScrCmd_415",
    "ScrCmd_416",
    "ScrCmd_417",
    "ScrCmd_418",
    "ScrCmd_419",
    "ScrCmd_420",
    "ScrCmd_421",
    "ScrCmd_422",
    "ScrCmd_CheckJohtoDexComplete",
    "ScrCmd_CheckNationalDexComplete",
    "ScrCmd_ShowCertificate",
    "ScrCmd_KenyaCheck",
    "ScrCmd_427",
    "ScrCmd_MonGiveMail",
    "ScrCmd_CountFossils",
    "ScrCmd_SetPhoneCall",
    "ScrCmd_RunPhoneCall",
    "ScrCmd_GetFossilPokemon",
    "ScrCmd_GetFossilMinimumAmount",
    "ScrCmd_PartyCountMonsAtOrBelowLevel",
    "ScrCmd_SurvivePoisoning",
    "ScrCmd_436",
    "ScrCmd_DebugWatch",
    "ScrCmd_GetStdMsgNaix",
    "ScrCmd_NonNPCMsgExtern",
    "ScrCmd_MsgBoxExtern",
    "ScrCmd_441",
    "ScrCmd_442",
    "ScrCmd_443",
    "ScrCmd_444",
    "ScrCmd_445",
    "ScrCmd_446",
    "ScrCmd_SafariZoneAction",
    "ScrCmd_448",
    "ScrCmd_449",
    "ScrCmd_450",
    "ScrCmd_451",
    "ScrCmd_452",
    "ScrCmd_453",
    "ScrCmd_454",
    "ScrCmd_455",
    "ScrCmd_456",
    "ScrCmd_MonGetNature",
    "ScrCmd_GetPartySlotWithNature",
    "ScrCmd_459",
    "ScrCmd_LoadPhoneDat",
    "ScrCmd_GetPhoneContactMsgIds",
    "ScrCmd_462",
    "ScrCmd_EnableMassOutbreaks",
    "ScrCmd_CreateRoamer",
    "ScrCmd_465",
    "ScrCmd_466",
    "ScrCmd_MoveRelearnerInit",
    "ScrCmd_MoveTutorInit",
    "ScrCmd_MoveRelearnerGetResult",
    "ScrCmd_LoadNPCTrade",
    "ScrCmd_GetOfferedSpecies",
    "ScrCmd_NPCTradeGetReqSpecies",
    "ScrCmd_NPCTradeExec",
    "ScrCmd_NPCTradeEnd",
    "ScrCmd_475",
    "ScrCmd_EnablePokedexFormDetection",
    "ScrCmd_NatDexFlagAction",
    "ScrCmd_MonGetRibbonCount",
    "ScrCmd_GetPartyRibbonCount",
    "ScrCmd_MonHasRibbon",
    "ScrCmd_GiveRibbon",
    "ScrCmd_BufferRibbonName",
    "ScrCmd_GetEVTotal",
    "ScrCmd_GetWeekday",
    "ScrCmd_StartBattleRegulationMenuTask",
    "ScrCmd_Dummy",
    "ScrCmd_PokeCenAnim",
    "ScrCmd_ElevatorAnim",
    "ScrCmd_MysteryGift",
    "ScrCmd_NopVar490",
    "ScrCmd_491",
    "ScrCmd_492",
    "ScrCmd_PromptEasyChat",
    "ScrCmd_494",
    "ScrCmd_GetGameVersion",
    "ScrCmd_GetPartyLead",
    "ScrCmd_GetMonTypes",
    "ScrCmd_PrimoPasswordCheck1",
    "ScrCmd_PrimoPasswordCheck2",
    "ScrCmd_500",
    "ScrCmd_501",
    "ScrCmd_502",
    "ScrCmd_LotoIDGet",
    "ScrCmd_LotoIDSearch",
    "ScrCmd_LotoIDSet",
    "ScrCmd_BufferBoxMonNick",
    "ScrCmd_CountPCEmptySpace",
    "ScrCmd_PalParkAction",
    "ScrCmd_509",
    "ScrCmd_510",
    "ScrCmd_PalParkScoreGet",
    "ScrCmd_PlayerMovementSavingSet",
    "ScrCmd_PlayerMovementSavingClear",
    "ScrCmd_HallOfFameAnim",
    "ScrCmd_AddSpecialGameStat",
    "ScrCmd_BufferFashionName",
    "ScrCmd_517",
    "ScrCmd_518",
    "ScrCmd_519",
    "ScrCmd_520",
    "ScrCmd_521",
    "ScrCmd_522",
    "ScrCmd_523",
    "ScrCmd_524",
    "ScrCmd_525",
    "ScrCmd_526",
    "ScrCmd_527",
    "ScrCmd_528",
    "ScrCmd_GetPartyLeadAlive",
    "ScrCmd_530",
    "ScrCmd_BufferBackgroundName",
    "ScrCmd_CheckCoinsImmediate",
    "ScrCmd_CheckGiveCoins",
    "ScrCmd_534",
    "ScrCmd_MonGetLevel",
    "ScrCmd_536",
    "ScrCmd_537",
    "ScrCmd_538",
    "ScrCmd_539",
    "ScrCmd_540",
    "ScrCmd_BufferIntEx",
    "ScrCmd_MonGetContestValue",
    "ScrCmd_543",
    "ScrCmd_544",
    "ScrCmd_545",
    "ScrCmd_546",
    "ScrCmd_547",
    "ScrCmd_548",
    "ScrCmd_549",
    "ScrCmd_550",
    "ScrCmd_551",
    "ScrCmd_552",
    "ScrCmd_553",
    "ScrCmd_554",
    "ScrCmd_555",
    "ScrCmd_556",
    "ScrCmd_CheckBattlePoints",
    "ScrCmd_UnionRoomAvatarIdxToSprite",
    "ScrCmd_559",
    "ScrCmd_560",
    "ScrCmd_ScreenShake",
    "ScrCmd_MultiBattle",
    "ScrCmd_563",
    "ScrCmd_564",
    "ScrCmd_565",
    "ScrCmd_566",
    "ScrCmd_GetDPPlPrizeItemIDAndCost",
    "ScrCmd_568",
    "ScrCmd_569",
    "ScrCmd_CheckCoinsVar",
    "ScrCmd_571",
    "ScrCmd_GetUniqueSealsQuantity",
    "ScrCmd_573",
    "ScrCmd_574",
    "ScrCmd_575",
    "ScrCmd_576",
    "ScrCmd_577",
    "ScrCmd_578",
    "ScrCmd_579",
    "ScrCmd_BufferSealName",
    "ScrCmd_LockLastTalked",
    "ScrCmd_582",
    "ScrCmd_583",
    "ScrCmd_PartyLegalCheck",
    "ScrCmd_585",
    "ScrCmd_586",
    "ScrCmd_587",
    "ScrCmd_LatiCaughtCheck",
    "ScrCmd_WildBattle",
    "ScrCmd_GetTrcardStars",
    "ScrCmd_591",
    "ScrCmd_592",
    "ScrCmd_ShowSaveStats",
    "ScrCmd_HideSaveStats",
    "ScrCmd_595",
    "ScrCmd_596",
    "ScrCmd_597",
    "ScrCmd_598",
    "ScrCmd_599",
    "ScrCmd_600",
    "ScrCmd_FollowMonFacePlayer",
    "ScrCmd_ToggleFollowingPokemonMovement",
    "ScrCmd_WaitFollowingPokemonMovement",
    "ScrCmd_FollowingPokemonMovement",
    "ScrCmd_605",
    "ScrCmd_606",
    "ScrCmd_607",
    "ScrCmd_608",
    "ScrCmd_609",
    "ScrCmd_610",
    "ScrCmd_Pokeathlon",
    "ScrCmd_GetNPCTradeUnusedFlag",
    "ScrCmd_GetPhoneContactRandomGiftBerry",
    "ScrCmd_GetPhoneContactGiftItem",
    "ScrCmd_CameronPhoto",
    "ScrCmd_CountSavedPhotos",
    "ScrCmd_OpenPhotoAlbum",
    "ScrCmd_PhotoAlbumIsFull",
    "ScrCmd_RocketCostumeFlagCheck",
    "ScrCmd_RocketCostumeFlagAction",
    "ScrCmd_PlaceStarterBallsInElmsLab",
    "ScrCmd_622",
    "ScrCmd_AnimApricornTree",
    "ScrCmd_ApricornTreeGetApricorn",
    "ScrCmd_GiveApricornFromTree",
    "ScrCmd_BufferApricornName",
    "ScrCmd_627",
    "ScrCmd_628",
    "ScrCmd_629",
    "ScrCmd_630",
    "ScrCmd_631",
    "ScrCmd_CountPartyMonsOfSpecies",
    "ScrCmd_633",
    "ScrCmd_634",
    "ScrCmd_635",
    "ScrCmd_636",
    "ScrCmd_637",
    "ScrCmd_638",
    "ScrCmd_639",
    "ScrCmd_640",
    "ScrCmd_SaveWipeExtraChunks",
    "ScrCmd_642",
    "ScrCmd_643",
    "ScrCmd_644",
    "ScrCmd_645",
    "ScrCmd_646",
    "ScrCmd_GetPartySlotWithSpecies",
    "ScrCmd_648",
    "ScrCmd_ScratchOffCard",
    "ScrCmd_ScratchOffCardEnd",
    "ScrCmd_GetScratchOffPrize",
    "ScrCmd_652",
    "ScrCmd_MoveTutorChooseMove",
    "ScrCmd_TutorMoveTeachInSlot",
    "ScrCmd_TutorMoveGetPrice",
    "ScrCmd_656",
    "ScrCmd_StatJudge",
    "ScrCmd_BufferStatName",
    "ScrCmd_SetMonForm",
    "ScrCmd_BufferTrainerName",
    "ScrCmd_661",
    "ScrCmd_662",
    "ScrCmd_663",
    "ScrCmd_664",
    "ScrCmd_665",
    "ScrCmd_666",
    "ScrCmd_667",
    "ScrCmd_BufferTypeName",
    "ScrCmd_GetItemQuantity",
    "ScrCmd_GetHiddenPowerType",
    "ScrCmd_SetFavoriteMon",
    "ScrCmd_GetFavoriteMon",
    "ScrCmd_GetOwnedRotomForms",
    "ScrCmd_CountTranformedRotomsInParty",
    "ScrCmd_UpdateRotomForm",
    "ScrCmd_GetPartyMonForm",
    "ScrCmd_677",
    "ScrCmd_678",
    "ScrCmd_679",
    "ScrCmd_AddSpecialGameStat2",
    "ScrCmd_681",
    "ScrCmd_682",
    "ScrCmd_GetStaticEncounterOutcome",
    "ScrCmd_684",
    "ScrCmd_GetPlayerXYZ",
    "ScrCmd_686",
    "ScrCmd_687",
    "ScrCmd_GetPartySlotWithFatefulEncounter",
    "ScrCmd_CommSanitizeParty",
    "ScrCmd_DaycareSanitizeMon",
    "ScrCmd_691",
    "ScrCmd_BufferBattleHallStreak",
    "ScrCmd_BattleHallCountUsedSpecies",
    "ScrCmd_BattleHallGetTotalStreak",
    "ScrCmd_695",
    "ScrCmd_696",
    "ScrCmd_697",
    "ScrCmd_FollowerPokeIsEventTrigger",
    "ScrCmd_699",
    "ScrCmd_700",
    "ScrCmd_MonHasItem",
    "ScrCmd_BattleTowerSetUpMultiBattle",
    "ScrCmd_SetPlayerVolume",
    "ScrCmd_704",
    "ScrCmd_705",
    "ScrCmd_706",
    "ScrCmd_CheckMonSeen",
    "ScrCmd_708",
    "ScrCmd_709",
    "ScrCmd_710",
    "ScrCmd_FollowMonInteract",
    "ScrCmd_712",
    "ScrCmd_AlphPuzzle",
    "ScrCmd_OpenAlphHiddenRoom",
    "ScrCmd_UpdateDaycareMonObjects",
    "ScrCmd_716",
    "ScrCmd_717",
    "ScrCmd_718",
    "ScrCmd_719",
    "ScrCmd_720",
    "ScrCmd_721",
    "ScrCmd_722",
    "ScrCmd_723",
    "ScrCmd_724",
    "ScrCmd_725",
    "ScrCmd_ProcessSoundplate",
    "ScrCmd_GetFollowPokePartyIndex",
    "ScrCmd_728",
    "ScrCmd_729",
    "ScrCmd_730",
    "ScrCmd_731",
    "ScrCmd_732",
    "ScrCmd_733",
    "ScrCmd_734",
    "ScrCmd_735",
    "ScrCmd_ClearKurtApricorn",
    "ScrCmd_737",
    "ScrCmd_GetTotalApricornCount",
    "ScrCmd_739",
    "ScrCmd_740",
    "ScrCmd_741",
    "ScrCmd_742",
    "ScrCmd_743",
    "ScrCmd_CreatePokeathlonFriendshipRoomStatues",
    "ScrCmd_BufferPokeathlonCourseName",
    "ScrCmd_TouchscreenMenuHide",
    "ScrCmd_TouchscreenMenuShow",
    "ScrCmd_GetMenuChoice",
    "ScrCmd_MenuInitStdGmm",
    "ScrCmd_MenuInit",
    "ScrCmd_MenuItemAdd",
    "ScrCmd_MenuExec",
    "ScrCmd_RockSmashItemCheck",
    "ScrCmd_TryHeadbuttEncounter",
    "ScrCmd_LegendCutsceneClearBellAnimBegin",
    "ScrCmd_LegendCutsceneClearBellAnimEnd",
    "ScrCmd_LegendCutsceneClearBellRiseFromBag",
    "ScrCmd_LegendCutsceneClearBellShimmer",
    "ScrCmd_LegendCutsceneLugiaEyeGlimmerEffect",
    "ScrCmd_760",
    "ScrCmd_LegendCutsceneMoveCameraTo",
    "ScrCmd_LegendCutscenePanCameraTo",
    "ScrCmd_LegendCutsceneWaitCameraPan",
    "ScrCmd_LegendCutsceneBirdFinalApproach",
    "ScrCmd_LegendCutsceneWavesOrLeavesEffectBegin",
    "ScrCmd_LegendCutsceneWavesOrLeavesEffectEnd",
    "ScrCmd_LegendCutsceneLugiaArrivesEffectBegin",
    "ScrCmd_LegendCutsceneLugiaArrivesEffectEnd",
    "ScrCmd_LegendCutsceneLugiaArrivesEffectCameraPan",
    "ScrCmd_CheckSeenAllLetterUnown",
    "ScrCmd_771",
    "ScrCmd_772",
    "ScrCmd_Cinematic",
    "ScrCmd_ShowLegendaryWing",
    "ScrCmd_775",
    "ScrCmd_GiveTogepiEgg",
    "ScrCmd_777",
    "ScrCmd_GiveSpikyEarPichu",
    "ScrCmd_RadioMusicIsPlaying",
    "ScrCmd_CasinoGame",
    "ScrCmd_KenyaCheckPartyOrMailbox",
    "ScrCmd_MartSell",
    "ScrCmd_SetFollowMonInhibitState",
    "ScrCmd_ScriptOverlayCmd",
    "ScrCmd_BugContestAction",
    "ScrCmd_BufferBugContestWinner",
    "ScrCmd_JudgeBugContest",
    "ScrCmd_BufferBugContestMonNick",
    "ScrCmd_BugContestGetTimeLeft",
    "ScrCmd_IsBugContestantRegistered",
    "ScrCmd_CheckSafariZoneChallengeCompleted",
    "ScrCmd_UpdateSafariZoneIGT",
    "ScrCmd_BankTransaction",
    "ScrCmd_CheckBankBalance",
    "ScrCmd_795",
    "ScrCmd_796",
    "ScrCmd_797",
    "ScrCmd_BufferRulesetName",
    "ScrCmd_799",
    "ScrCmd_800",
    "ScrCmd_801",
    "ScrCmd_802",
    "ScrCmd_803",
    "ScrCmd_804",
    "ScrCmd_805",
    "ScrCmd_806",
    "ScrCmd_SetTrainerHouseSprite",
    "ScrCmd_808",
    "ScrCmd_ShowTrainerHouseIntroMessage",
    "ScrCmd_810",
    "ScrCmd_811",
    "ScrCmd_812",
    "ScrCmd_MomGiftCheck",
    "ScrCmd_814",
    "ScrCmd_815",
    "ScrCmd_UnownCircle",
    "ScrCmd_817",
    "ScrCmd_MystriStageGymmickInit",
    "ScrCmd_819",
    "ScrCmd_820",
    "ScrCmd_GetBuenasPassword",
    "ScrCmd_822",
    "ScrCmd_823",
    "ScrCmd_824",
    "ScrCmd_GetShinyLeafCount",
    "ScrCmd_TryGiveShinyLeafCrown",
    "ScrCmd_GetPartyMonForm2",
    "ScrCmd_MonAddContestValue",
    "ScrCmd_829",
    "ScrCmd_830",
    "ScrCmd_831",
    "ScrCmd_832",
    "ScrCmd_833",
    "ScrCmd_834",
    "ScrCmd_835",
    "ScrCmd_CheckKyogreGroudonInParty",
    "ScrCmd_837",
    "ScrCmd_BankOrWalletIsFull",
    "ScrCmd_SysSetSleepFlag",
    "ScrCmd_840",
    "ScrCmd_841",
    "ScrCmd_842",
    "ScrCmd_BufferItemNameIndef",
    "ScrCmd_BufferItemNamePlural",
    "ScrCmd_BufferPartyMonSpeciesNameIndef",
    "ScrCmd_BufferSpeciesNameIndef",
    "ScrCmd_BufferDPPtFriendStarterSpeciesNameIndef",
    "ScrCmd_BufferFashionNameIndef",
    "ScrCmd_BufferTrainerClassNameIndef",
    "ScrCmd_BufferSealNamePlural",
    "ScrCmd_Capitalize",
    "ScrCmd_BufferDeptStoreFloorNo",
];

/// Operand sizes in bytes, in order, from `asm/macros/script.inc`.
/// The five variable-layout commands (400–402, 465, 489) list their
/// *leading* fixed operand only; [`Opcode::is_variable_layout`] and
/// [`decode_at`] handle the rest.
static OPERANDS: [&[u8]; OPCODE_COUNT] = [
    &[], // 0 Noop
    &[], // 1 Dummy
    &[], // 2 End
    &[2, 2], // 3 Wait
    &[1, 1], // 4 LoadByte
    &[1, 4], // 5 LoadWord
    &[1, 4], // 6 LoadByteFromAddr
    &[4, 1], // 7 WriteByteToAddr
    &[4, 1], // 8 SetPtrByte
    &[1, 1], // 9 CopyLocal
    &[4, 4], // 10 CopyByte
    &[1, 1], // 11 CompareLocalToLocal
    &[1, 1], // 12 CompareLocalToValue
    &[1, 4], // 13 CompareLocalToAddr
    &[4, 1], // 14 CompareAddrToLocal
    &[4, 1], // 15 CompareAddrToValue
    &[4, 4], // 16 CompareAddrToAddr
    &[2, 2], // 17 CompareVarToValue
    &[2, 2], // 18 CompareVarToVar
    &[2], // 19 RunScript
    &[2], // 20 CallStd
    &[], // 21 RestartCurrentScript
    &[4], // 22 GoTo
    &[1, 4], // 23 ObjectGoTo
    &[1, 4], // 24 BGGoTo
    &[1, 4], // 25 DirectionGoTo
    &[4], // 26 Call
    &[], // 27 Return
    &[1, 4], // 28 GoToIf
    &[1, 4], // 29 CallIf
    &[2], // 30 SetFlag
    &[2], // 31 ClearFlag
    &[2], // 32 CheckFlag
    &[2], // 33 SetFlagVar
    &[2], // 34 ClearFlagVar
    &[2, 2], // 35 CheckFlagVar
    &[2], // 36 SetTrainerFlag
    &[2], // 37 ClearTrainerFlag
    &[2], // 38 CheckTrainerFlag
    &[2, 2], // 39 AddVar
    &[2, 2], // 40 SubVar
    &[2, 2], // 41 SetVar
    &[2, 2], // 42 CopyVar
    &[2, 2], // 43 SetOrCopyVar
    &[1], // 44 NonNPCMsg
    &[1], // 45 NPCMsg
    &[2], // 46 NonNPCMsgVar
    &[2], // 47 NPCMsgVar
    &[1], // 48 ScrCmd_048
    &[], // 49 WaitABPress
    &[], // 50 WaitButton
    &[], // 51 WaitButtonOrDpad
    &[], // 52 OpenMsg
    &[], // 53 CloseMsg
    &[], // 54 HoldMsg
    &[1, 1, 2, 2], // 55 DirectionSignpost
    &[1, 2], // 56 SetSignpostMap
    &[1], // 57 SetSignpostAction
    &[], // 58 WaitSignpostAction
    &[1, 2], // 59 TrainerTips
    &[2], // 60 WaitSignpost
    &[], // 61 ScrCmd_061
    &[1, 1, 1, 1, 1, 1], // 62 ScrCmd_062
    &[2], // 63 YesNo
    &[1, 1, 1, 1, 2], // 64 ScrCmd_064
    &[1, 1, 1, 1, 2], // 65 ScrCmd_065
    &[1, 1], // 66 ScrCmd_066
    &[], // 67 ScrCmd_067
    &[1, 1, 1, 1, 2], // 68 ScrCmd_068
    &[1, 1, 1, 1, 2], // 69 ScrCmd_069
    &[2, 2, 2], // 70 ScrCmd_070
    &[], // 71 ScrCmd_071
    &[1], // 72 ScrCmd_072
    &[2], // 73 PlaySE
    &[2], // 74 StopSE
    &[2], // 75 WaitSE
    &[2, 2], // 76 PlayCry
    &[], // 77 WaitCry
    &[2], // 78 PlayFanfare
    &[], // 79 WaitFanfare
    &[2], // 80 PlayBGM
    &[2], // 81 StopBGM
    &[], // 82 ResetBGM
    &[2], // 83 ScrCmd_083
    &[2, 2], // 84 FadeOutBGM
    &[2], // 85 FadeInBGM
    &[1, 1], // 86 ScrCmd_086
    &[2], // 87 TempBGM
    &[1], // 88 ScrCmd_088
    &[2], // 89 ChatotHasCry
    &[2], // 90 ChatotStartRecording
    &[], // 91 ChatotStopRecording
    &[], // 92 ChatotSaveRecording
    &[], // 93 ScrCmd_093
    &[2, 4], // 94 ApplyMovement
    &[], // 95 WaitMovement
    &[], // 96 LockAll
    &[], // 97 ReleaseAll
    &[2], // 98 Lock
    &[2], // 99 Release
    &[2], // 100 ShowPerson
    &[2], // 101 HidePerson
    &[2, 2], // 102 ScrCmd_102
    &[], // 103 ScrCmd_103
    &[], // 104 FacePlayer
    &[2, 2], // 105 GetPlayerCoords
    &[2, 2, 2], // 106 GetPersonCoords
    &[2, 2, 2], // 107 ScrCmd_107
    &[2, 1], // 108 ScrCmd_108
    &[2, 2], // 109 ScrCmd_109
    &[4], // 110 AddMoney
    &[4], // 111 SubMoneyImmediate
    &[2, 4], // 112 HasEnoughMoneyImmediate
    &[2, 2], // 113 ShowMoneyBox
    &[], // 114 HideMoneyBox
    &[], // 115 UpdateMoneyBox
    &[1, 2, 2], // 116 ScrCmd_116
    &[], // 117 ScrCmd_117
    &[1], // 118 ScrCmd_118
    &[2], // 119 GetCoinAmount
    &[2], // 120 GiveCoins
    &[2], // 121 TakeCoins
    &[2], // 122 GiveAthletePoints
    &[2], // 123 TakeAthletePoints
    &[2, 2], // 124 CheckAthletePoints
    &[2, 2, 2], // 125 GiveItem
    &[2, 2, 2], // 126 TakeItem
    &[2, 2, 2], // 127 HasSpaceForItem
    &[2, 2, 2], // 128 HasItem
    &[2, 2], // 129 ItemIsTMOrHM
    &[2, 2], // 130 GetItemPocket
    &[2], // 131 SetStarterChoice
    &[1, 1], // 132 GenderMsgBox
    &[2, 2], // 133 GetSealQuantity
    &[2, 2], // 134 GiveOrTakeSeal
    &[2, 2, 2], // 135 GiveRandomSeal
    &[2, 2], // 136 ScrCmd_136
    &[2, 2, 2, 2, 2, 2], // 137 GiveMon
    &[2, 2], // 138 GiveEgg
    &[2, 2, 2], // 139 SetMonMove
    &[2, 2, 2], // 140 MonHasMove
    &[2, 2], // 141 GetPartySlotWithMove
    &[2, 2], // 142 GetPhoneBookRematch
    &[2], // 143 NameRival
    &[2], // 144 GetFriendSprite
    &[1], // 145 RegisterPokegearCard
    &[2], // 146 RegisterGearNumber
    &[2, 2], // 147 CheckRegisteredPhoneNumber
    &[1, 1], // 148 ScrCmd_148
    &[1], // 149 UnsetPhoneCallTrigger
    &[], // 150 RestoreOverworld
    &[], // 151 ScrCmd_151
    &[], // 152 ScrCmd_152
    &[2], // 153 ScrCmd_153
    &[2, 2, 2], // 154 ScrCmd_154
    &[2, 2], // 155 ScrCmd_155
    &[], // 156 ScrCmd_156
    &[], // 157 TownMap
    &[1], // 158 ScrCmd_158
    &[], // 159 ScrCmd_159
    &[], // 160 ScrCmd_160
    &[], // 161 ScrCmd_161
    &[], // 162 ScrCmd_162
    &[2], // 163 HOFCredits
    &[], // 164 ScrCmd_164
    &[2, 2], // 165 ScrCmd_165
    &[2], // 166 ScrCmd_166
    &[], // 167 ChooseStarter
    &[2], // 168 GetTrainerPathToPlayer
    &[2, 2], // 169 TrainerStepTowardsPlayer
    &[2], // 170 GetTrainerEyeType
    &[2, 2], // 171 GetEyeTrainerNum
    &[2], // 172 NamePlayer
    &[2, 2], // 173 NicknameInput
    &[2, 2, 2, 2], // 174 FadeScreen
    &[], // 175 WaitFade
    &[2, 2, 2, 2, 2], // 176 Warp
    &[2], // 177 RockClimb
    &[2], // 178 Surf
    &[2], // 179 Waterfall
    &[2, 2, 2], // 180 ScrCmd_180
    &[], // 181 FlashEffect
    &[2], // 182 Whirlpool
    &[2], // 183 ScrCmd_183
    &[2], // 184 PlayerOnBikeCheck
    &[1], // 185 PlayerOnBikeSet
    &[1], // 186 SetBikeStateLock
    &[2], // 187 GetPlayerState
    &[2], // 188 SetAvatarBits
    &[], // 189 UpdateAvatarState
    &[1], // 190 BufferPlayersName
    &[1], // 191 BufferRivalsName
    &[1], // 192 BufferFriendsName
    &[1, 2], // 193 BufferMonSpeciesName
    &[1, 2], // 194 BufferItemName
    &[1, 2], // 195 BufferPocketName
    &[1, 2], // 196 BufferTMHMMoveName
    &[1, 2], // 197 BufferMoveName
    &[1, 2], // 198 BufferInt
    &[1, 2], // 199 BufferPartyMonNick
    &[1, 2], // 200 BufferTrainerClassName
    &[1], // 201 BufferPlayerUnionAvatarClassName
    &[1, 2, 2, 1], // 202 BufferSpeciesName
    &[1], // 203 BufferStarterSpeciesName
    &[1], // 204 BufferDPPtRivalStarterSpeciesName
    &[1], // 205 BufferDPPtFriendStarterSpeciesName
    &[2], // 206 GetStarterChoice
    &[1, 2], // 207 BufferDecorationName
    &[1, 2], // 208 ScrCmd_208
    &[1, 2], // 209 ScrCmd_209
    &[1, 2], // 210 BufferMapSecName
    &[2, 2], // 211 ScrCmd_211
    &[2], // 212 GetTrainerNum
    &[2, 2, 1, 1], // 213 TrainerBattle
    &[2, 2], // 214 TrainerMessage
    &[2, 2, 2], // 215 GetTrainerMsgParams
    &[2, 2, 2], // 216 GetRematchMsgParams
    &[2], // 217 TrainerIsDoubleBattle
    &[2], // 218 EncounterMusic
    &[], // 219 WhiteOut
    &[2], // 220 CheckBattleWon
    &[2, 1], // 221 StaticWildWonOrCaughtCheck
    &[2], // 222 PartyCheckForDouble
    &[], // 223 ScrCmd_223
    &[], // 224 ScrCmd_224
    &[4], // 225 GoToIfTrainerDefeated
    &[2, 2, 2, 2], // 226 ScrCmd_226
    &[2, 2, 2, 2], // 227 ScrCmd_227
    &[2], // 228 ScrCmd_228
    &[2], // 229 ScrCmd_229
    &[], // 230 ScrCmd_230
    &[], // 231 ScrCmd_231
    &[2], // 232 ScrCmd_232
    &[2], // 233 ScrCmd_233
    &[2, 2, 2, 2], // 234 ScrCmd_234
    &[2], // 235 ScrCmd_235
    &[2], // 236 ScrCmd_236
    &[], // 237 ScrCmd_237
    &[2], // 238 PartyHasPokerus
    &[2, 2], // 239 MonGetGender
    &[2, 2, 2, 2, 2], // 240 SetDynamicWarp
    &[2], // 241 GetDynamicWarpFloorNo
    &[1, 1, 2, 2], // 242 ElevatorCurFloorBox
    &[2], // 243 CountJohtoDexSeen
    &[2], // 244 CountJohtoDexOwned
    &[2], // 245 CountNationalDexSeen
    &[2], // 246 CountNationalDexOwned
    &[], // 247 ScrCmd_247
    &[1, 2, 2], // 248 GetDexEvalResult
    &[2, 2], // 249 RocketTrapBattle
    &[2, 2], // 250 ScrCmd_250
    &[], // 251 CatchingTutorial
    &[], // 252 ScrCmd_252
    &[2], // 253 GetSaveFileState
    &[2], // 254 SaveGameNormal
    &[2, 2], // 255 ScrCmd_255
    &[2], // 256 ScrCmd_256
    &[2], // 257 ScrCmd_257
    &[], // 258 ScrCmd_258
    &[2], // 259 ScrCmd_259
    &[2], // 260 ScrCmd_260
    &[2], // 261 ScrCmd_261
    &[], // 262 ScrCmd_262
    &[], // 263 ScrCmd_263
    &[2], // 264 ScrCmd_264
    &[], // 265 ScrCmd_265
    &[], // 266 ScrCmd_266
    &[2, 2], // 267 ScrCmd_267
    &[2], // 268 ScrCmd_268
    &[2], // 269 ScrCmd_269
    &[], // 270 ScrCmd_270
    &[2, 2], // 271 ScrCmd_271
    &[2], // 272 ScrCmd_272
    &[2], // 273 ScrCmd_273
    &[2, 2], // 274 ScrCmd_274
    &[2], // 275 MartBuy
    &[2], // 276 SpecialMartBuy
    &[2], // 277 DecorationMart
    &[2], // 278 SealMart
    &[], // 279 OverworldWhiteOut
    &[2], // 280 SetSpawn
    &[2], // 281 GetPlayerGender
    &[], // 282 HealParty
    &[], // 283 ScrCmd_283
    &[], // 284 ScrCmd_284
    &[2], // 285 ScrCmd_285
    &[], // 286 ScrCmd_286
    &[], // 287 BufferUnionRoomAvatarChoices
    &[2, 2], // 288 UnionRoomAvatarIdxToTrainerClass
    &[2], // 289 ScrCmd_289
    &[2], // 290 CheckPokedex
    &[], // 291 GivePokedex
    &[2], // 292 CheckRunningShoes
    &[], // 293 GiveRunningShoes
    &[2, 2], // 294 CheckBadge
    &[2], // 295 GiveBadge
    &[2], // 296 CountBadges
    &[2], // 297 ScrCmd_297
    &[], // 298 ScrCmd_298
    &[2], // 299 CheckEscortMode
    &[], // 300 SetEscortMode
    &[], // 301 ClearEscortMode
    &[2], // 302 CheckStepTakenFlag
    &[], // 303 SetStepTakenFlag
    &[], // 304 GetStepTakenFlag
    &[2], // 305 CheckGameClearFlag
    &[], // 306 SetGameClearFlag
    &[2, 2, 2, 2, 1], // 307 ScrCmd_307
    &[1], // 308 ScrCmd_308
    &[1], // 309 ScrCmd_309
    &[1], // 310 ScrCmd_310
    &[1], // 311 ScrCmd_311
    &[], // 312 BufferDaycareMonNicks
    &[2], // 313 GetDaycareState
    &[], // 314 EcruteakGymInit
    &[], // 315 ScrCmd_315
    &[], // 316 ScrCmd_316
    &[1], // 317 ScrCmd_317
    &[], // 318 CianwoodGymInit
    &[2], // 319 CianwoodGymTurnWinch
    &[], // 320 VermilionGymInit
    &[1, 1], // 321 VermilionGymLockAction
    &[1, 2], // 322 VermilionGymCanCheck
    &[], // 323 ResampleVermilionGymCans
    &[], // 324 VioletGymInit
    &[], // 325 VioletGymElevator
    &[], // 326 AzaleaGymInit
    &[1], // 327 AzaleaGymSpinarak
    &[1], // 328 AzaleaGymSwitch
    &[], // 329 BlackthornGymInit
    &[], // 330 FuchsiaGymInit
    &[], // 331 ViridianGymInit
    &[2], // 332 GetPartyCount
    &[1], // 333 ScrCmd_333
    &[2], // 334 ScrCmd_334
    &[2, 2], // 335 ScrCmd_335
    &[1, 2, 2], // 336 BufferBerryName
    &[1, 2], // 337 BufferNatureName
    &[2, 2, 2], // 338 MovePerson
    &[2, 2, 2, 2, 2], // 339 MovePersonFacing
    &[2, 2], // 340 SetObjectMovementType
    &[2, 2], // 341 SetObjectFacing
    &[2, 2, 2], // 342 MoveWarp
    &[2, 2, 2], // 343 MoveBGEvent
    &[2, 2], // 344 ScrCmd_344
    &[], // 345 AddWaitingIcon
    &[], // 346 RemoveWaitingIcon
    &[2], // 347 ScrCmd_347
    &[2], // 348 WaitButtonOrDelay
    &[], // 349 PartySelectUI
    &[], // 350 ScrCmd_350
    &[2], // 351 GetPartySelection
    &[1, 2, 2], // 352 PokemonSummaryScreen
    &[1, 2], // 353 GetMoveSelection
    &[2, 2], // 354 GetPartyMonSpecies
    &[2, 2], // 355 PartyMonIsMine
    &[2], // 356 PartyCountNotEgg
    &[2, 2], // 357 CountAliveMons
    &[2], // 358 CountAliveMonsAndPC
    &[2], // 359 PartyCountEgg
    &[2], // 360 SubMoneyVar
    &[2, 2], // 361 RetrieveDaycareMon
    &[1, 1, 2], // 362 GiveLoanMon
    &[1, 2, 2], // 363 CheckReturnLoanMon
    &[2], // 364 ReturnLoanMon
    &[], // 365 ResetDaycareEgg
    &[], // 366 GiveDaycareEgg
    &[2, 2], // 367 BufferDaycareWithdrawCost
    &[2, 2], // 368 HasEnoughMoneyVar
    &[], // 369 EggHatchAnim
    &[1], // 370 ScrCmd_370
    &[2, 2], // 371 BufferDaycareMonGrowth
    &[2], // 372 GetTailDaycareMonSpeciesAndNick
    &[2], // 373 PutMonInDaycare
    &[2], // 374 ScrCmd_374
    &[2], // 375 MakeObjectVisible
    &[], // 376 ScrCmd_376
    &[2], // 377 ScrCmd_377
    &[2, 2], // 378 ViewRankings
    &[2], // 379 ScrCmd_379
    &[2, 2], // 380 Random
    &[2, 2], // 381 ScrCmd_381
    &[2, 2], // 382 MonGetFriendship
    &[2, 2], // 383 MonAddFriendship
    &[2, 2], // 384 MonSubtractFriendship
    &[2, 2, 2, 2], // 385 BufferDaycareMonStats
    &[2], // 386 GetPlayerFacing
    &[2], // 387 GetDaycareCompatibility
    &[2], // 388 CheckDaycareEgg
    &[2, 2], // 389 PlayerHasSpecies
    &[2, 2], // 390 SizeRecordCompare
    &[2], // 391 SizeRecordUpdate
    &[2, 2, 2], // 392 BufferMonSize
    &[2, 2, 2], // 393 BufferRecordSize
    &[2], // 394 ScrCmd_394
    &[2], // 395 ScrCmd_395
    &[2, 2], // 396 CountMonMoves
    &[2, 2], // 397 MonForgetMove
    &[2, 2, 2], // 398 MonGetMove
    &[1, 2, 2], // 399 BufferPartyMonMoveName
    &[1], // 400 StrengthFlagAction
    &[1], // 401 FlashAction
    &[1], // 402 DefogAction
    &[2, 2], // 403 ScrCmd_403
    &[2, 2, 2], // 404 ScrCmd_404
    &[2, 2, 2], // 405 ScrCmd_405
    &[2], // 406 ScrCmd_406
    &[2, 2], // 407 ScrCmd_407
    &[2, 2], // 408 ScrCmd_408
    &[], // 409 ScrCmd_409
    &[2, 2], // 410 ScrCmd_410
    &[], // 411 ScrCmd_411
    &[2, 2, 2], // 412 ScrCmd_412
    &[2, 2, 2, 2], // 413 ScrCmd_413
    &[2], // 414 ScrCmd_414
    &[2], // 415 ScrCmd_415
    &[2, 2, 2], // 416 ScrCmd_416
    &[2, 2], // 417 ScrCmd_417
    &[2, 2], // 418 ScrCmd_418
    &[2], // 419 ScrCmd_419
    &[2], // 420 ScrCmd_420
    &[2, 2, 2], // 421 ScrCmd_421
    &[2, 2, 2, 1], // 422 ScrCmd_422
    &[2], // 423 CheckJohtoDexComplete
    &[2], // 424 CheckNationalDexComplete
    &[2], // 425 ShowCertificate
    &[2, 2, 1], // 426 KenyaCheck
    &[2], // 427 ScrCmd_427
    &[2], // 428 MonGiveMail
    &[2], // 429 CountFossils
    &[2, 2, 2], // 430 SetPhoneCall
    &[], // 431 RunPhoneCall
    &[2, 2], // 432 GetFossilPokemon
    &[2, 2, 2], // 433 GetFossilMinimumAmount
    &[2, 2], // 434 PartyCountMonsAtOrBelowLevel
    &[2, 2], // 435 SurvivePoisoning
    &[], // 436 ScrCmd_436
    &[2], // 437 DebugWatch
    &[2, 2], // 438 GetStdMsgNaix
    &[2, 2], // 439 NonNPCMsgExtern
    &[2, 2], // 440 MsgBoxExtern
    &[2, 2, 2, 2], // 441 ScrCmd_441
    &[2, 2, 2, 2], // 442 ScrCmd_442
    &[1], // 443 ScrCmd_443
    &[1, 2, 2, 1], // 444 ScrCmd_444
    &[2], // 445 ScrCmd_445
    &[2], // 446 ScrCmd_446
    &[1, 1], // 447 SafariZoneAction
    &[2, 2, 2, 2, 2], // 448 ScrCmd_448
    &[], // 449 ScrCmd_449
    &[], // 450 ScrCmd_450
    &[2], // 451 ScrCmd_451
    &[2, 2], // 452 ScrCmd_452
    &[], // 453 ScrCmd_453
    &[], // 454 ScrCmd_454
    &[], // 455 ScrCmd_455
    &[1], // 456 ScrCmd_456
    &[2, 2], // 457 MonGetNature
    &[2, 2], // 458 GetPartySlotWithNature
    &[], // 459 ScrCmd_459
    &[2, 2], // 460 LoadPhoneDat
    &[1, 2, 2], // 461 GetPhoneContactMsgIds
    &[2], // 462 ScrCmd_462
    &[], // 463 EnableMassOutbreaks
    &[1], // 464 CreateRoamer
    &[2], // 465 ScrCmd_465
    &[2, 2], // 466 ScrCmd_466
    &[2], // 467 MoveRelearnerInit
    &[2, 2], // 468 MoveTutorInit
    &[2], // 469 MoveRelearnerGetResult
    &[1], // 470 LoadNPCTrade
    &[2], // 471 GetOfferedSpecies
    &[2], // 472 NPCTradeGetReqSpecies
    &[2], // 473 NPCTradeExec
    &[], // 474 NPCTradeEnd
    &[], // 475 ScrCmd_475
    &[], // 476 EnablePokedexFormDetection
    &[1, 2], // 477 NatDexFlagAction
    &[2, 2], // 478 MonGetRibbonCount
    &[2], // 479 GetPartyRibbonCount
    &[2, 2, 2], // 480 MonHasRibbon
    &[2, 2], // 481 GiveRibbon
    &[1, 2], // 482 BufferRibbonName
    &[2, 2], // 483 GetEVTotal
    &[2], // 484 GetWeekday
    &[2], // 485 StartBattleRegulationMenuTask
    &[], // 486 Dummy486
    &[2], // 487 PokeCenAnim
    &[2, 2], // 488 ElevatorAnim
    &[2], // 489 MysteryGift
    &[2], // 490 NopVar490
    &[2], // 491 ScrCmd_491
    &[2, 2, 2], // 492 ScrCmd_492
    &[2, 2, 2], // 493 PromptEasyChat
    &[2, 2], // 494 ScrCmd_494
    &[2], // 495 GetGameVersion
    &[2], // 496 GetPartyLead
    &[2, 2, 2], // 497 GetMonTypes
    &[2, 2, 2, 2, 2], // 498 PrimoPasswordCheck1
    &[2, 2, 2, 2, 2], // 499 PrimoPasswordCheck2
    &[1], // 500 ScrCmd_500
    &[1], // 501 ScrCmd_501
    &[1], // 502 ScrCmd_502
    &[2], // 503 LotoIDGet
    &[2, 2, 2, 2], // 504 LotoIDSearch
    &[], // 505 LotoIDSet
    &[1, 2], // 506 BufferBoxMonNick
    &[2], // 507 CountPCEmptySpace
    &[2], // 508 PalParkAction
    &[2], // 509 ScrCmd_509
    &[], // 510 ScrCmd_510
    &[2, 2], // 511 PalParkScoreGet
    &[], // 512 PlayerMovementSavingSet
    &[], // 513 PlayerMovementSavingClear
    &[2], // 514 HallOfFameAnim
    &[2], // 515 AddSpecialGameStat
    &[1, 2], // 516 BufferFashionName
    &[2, 2], // 517 ScrCmd_517
    &[2], // 518 ScrCmd_518
    &[2], // 519 ScrCmd_519
    &[], // 520 ScrCmd_520
    &[], // 521 ScrCmd_521
    &[2], // 522 ScrCmd_522
    &[2, 2, 2, 2, 2], // 523 ScrCmd_523
    &[2, 2, 2], // 524 ScrCmd_524
    &[2], // 525 ScrCmd_525
    &[2], // 526 ScrCmd_526
    &[2], // 527 ScrCmd_527
    &[2], // 528 ScrCmd_528
    &[2], // 529 GetPartyLeadAlive
    &[2, 1], // 530 ScrCmd_530
    &[1, 2], // 531 BufferBackgroundName
    &[2, 4], // 532 CheckCoinsImmediate
    &[2, 2], // 533 CheckGiveCoins
    &[2], // 534 ScrCmd_534
    &[2, 2], // 535 MonGetLevel
    &[2, 2], // 536 ScrCmd_536
    &[], // 537 ScrCmd_537
    &[2, 2], // 538 ScrCmd_538
    &[2], // 539 ScrCmd_539
    &[2], // 540 ScrCmd_540
    &[1, 2, 1, 1], // 541 BufferIntEx
    &[2, 2, 2], // 542 MonGetContestValue
    &[2], // 543 ScrCmd_543
    &[2, 2], // 544 ScrCmd_544
    &[2], // 545 ScrCmd_545
    &[1, 2], // 546 ScrCmd_546
    &[2], // 547 ScrCmd_547
    &[], // 548 ScrCmd_548
    &[2], // 549 ScrCmd_549
    &[2], // 550 ScrCmd_550
    &[2], // 551 ScrCmd_551
    &[2, 2], // 552 ScrCmd_552
    &[1, 2], // 553 ScrCmd_553
    &[2], // 554 ScrCmd_554
    &[2], // 555 ScrCmd_555
    &[2], // 556 ScrCmd_556
    &[2, 2], // 557 CheckBattlePoints
    &[2, 2], // 558 UnionRoomAvatarIdxToSprite
    &[2, 2], // 559 ScrCmd_559
    &[2, 2], // 560 ScrCmd_560
    &[2, 2, 2, 2], // 561 ScreenShake
    &[2, 2, 2, 1], // 562 MultiBattle
    &[2, 2, 2], // 563 ScrCmd_563
    &[2], // 564 ScrCmd_564
    &[2], // 565 ScrCmd_565
    &[], // 566 ScrCmd_566
    &[2, 2, 2], // 567 GetDPPlPrizeItemIDAndCost
    &[2, 2], // 568 ScrCmd_568
    &[2], // 569 ScrCmd_569
    &[2, 2], // 570 CheckCoinsVar
    &[2, 2, 2, 2, 2], // 571 ScrCmd_571
    &[2], // 572 GetUniqueSealsQuantity
    &[], // 573 ScrCmd_573
    &[2, 2], // 574 ScrCmd_574
    &[2, 2], // 575 ScrCmd_575
    &[2], // 576 ScrCmd_576
    &[], // 577 ScrCmd_577
    &[], // 578 ScrCmd_578
    &[], // 579 ScrCmd_579
    &[1, 2], // 580 BufferSealName
    &[], // 581 LockLastTalked
    &[2, 2, 2], // 582 ScrCmd_582
    &[2, 1], // 583 ScrCmd_583
    &[2], // 584 PartyLegalCheck
    &[], // 585 ScrCmd_585
    &[2], // 586 ScrCmd_586
    &[], // 587 ScrCmd_587
    &[2], // 588 LatiCaughtCheck
    &[2, 2, 1], // 589 WildBattle
    &[2], // 590 GetTrcardStars
    &[], // 591 ScrCmd_591
    &[2], // 592 ScrCmd_592
    &[], // 593 ShowSaveStats
    &[], // 594 HideSaveStats
    &[1], // 595 ScrCmd_595
    &[2], // 596 ScrCmd_596
    &[], // 597 ScrCmd_597
    &[2], // 598 ScrCmd_598
    &[], // 599 ScrCmd_599
    &[], // 600 ScrCmd_600
    &[], // 601 FollowMonFacePlayer
    &[2], // 602 ToggleFollowingPokemonMovement
    &[], // 603 WaitFollowingPokemonMovement
    &[2], // 604 FollowingPokemonMovement
    &[1, 1], // 605 ScrCmd_605
    &[], // 606 ScrCmd_606
    &[], // 607 ScrCmd_607
    &[], // 608 ScrCmd_608
    &[], // 609 ScrCmd_609
    &[2], // 610 ScrCmd_610
    &[1, 1, 2, 2, 2, 2, 2], // 611 Pokeathlon
    &[2], // 612 GetNPCTradeUnusedFlag
    &[2], // 613 GetPhoneContactRandomGiftBerry
    &[2], // 614 GetPhoneContactGiftItem
    &[2], // 615 CameronPhoto
    &[2], // 616 CountSavedPhotos
    &[], // 617 OpenPhotoAlbum
    &[2], // 618 PhotoAlbumIsFull
    &[2], // 619 RocketCostumeFlagCheck
    &[1], // 620 RocketCostumeFlagAction
    &[], // 621 PlaceStarterBallsInElmsLab
    &[2, 2], // 622 ScrCmd_622
    &[2], // 623 AnimApricornTree
    &[2], // 624 ApricornTreeGetApricorn
    &[2, 2, 2], // 625 GiveApricornFromTree
    &[1, 2], // 626 BufferApricornName
    &[1], // 627 ScrCmd_627
    &[2, 2], // 628 ScrCmd_628
    &[], // 629 ScrCmd_629
    &[2], // 630 ScrCmd_630
    &[2, 2, 2], // 631 ScrCmd_631
    &[2, 2], // 632 CountPartyMonsOfSpecies
    &[2, 2, 2], // 633 ScrCmd_633
    &[2, 2], // 634 ScrCmd_634
    &[2, 2], // 635 ScrCmd_635
    &[2], // 636 ScrCmd_636
    &[2, 2, 2], // 637 ScrCmd_637
    &[2, 2, 2], // 638 ScrCmd_638
    &[2, 2, 2], // 639 ScrCmd_639
    &[2], // 640 ScrCmd_640
    &[], // 641 SaveWipeExtraChunks
    &[2], // 642 ScrCmd_642
    &[2, 2, 2], // 643 ScrCmd_643
    &[2, 2, 2], // 644 ScrCmd_644
    &[2, 2, 2], // 645 ScrCmd_645
    &[2], // 646 ScrCmd_646
    &[2, 2], // 647 GetPartySlotWithSpecies
    &[2, 2, 2, 2, 2], // 648 ScrCmd_648
    &[], // 649 ScratchOffCard
    &[], // 650 ScratchOffCardEnd
    &[2, 2, 2], // 651 GetScratchOffPrize
    &[2, 2, 2], // 652 ScrCmd_652
    &[2, 2, 2, 2], // 653 MoveTutorChooseMove
    &[2, 2, 2], // 654 TutorMoveTeachInSlot
    &[2, 2], // 655 TutorMoveGetPrice
    &[2, 2], // 656 ScrCmd_656
    &[2, 2, 2, 2], // 657 StatJudge
    &[1, 2], // 658 BufferStatName
    &[2, 2], // 659 SetMonForm
    &[1, 2], // 660 BufferTrainerName
    &[1, 4, 1, 1], // 661 ScrCmd_661
    &[2, 2, 2], // 662 ScrCmd_662
    &[2], // 663 ScrCmd_663
    &[], // 664 ScrCmd_664
    &[2], // 665 ScrCmd_665
    &[2], // 666 ScrCmd_666
    &[2], // 667 ScrCmd_667
    &[1, 2], // 668 BufferTypeName
    &[2, 2], // 669 GetItemQuantity
    &[2, 2], // 670 GetHiddenPowerType
    &[], // 671 SetFavoriteMon
    &[2, 2, 2], // 672 GetFavoriteMon
    &[2, 2, 2, 2, 2], // 673 GetOwnedRotomForms
    &[2, 2], // 674 CountTranformedRotomsInParty
    &[2, 2, 2, 2], // 675 UpdateRotomForm
    &[2, 2], // 676 GetPartyMonForm
    &[2, 2], // 677 ScrCmd_677
    &[2, 2], // 678 ScrCmd_678
    &[], // 679 ScrCmd_679
    &[2], // 680 AddSpecialGameStat2
    &[2], // 681 ScrCmd_681
    &[2], // 682 ScrCmd_682
    &[2], // 683 GetStaticEncounterOutcome
    &[2], // 684 ScrCmd_684
    &[2, 2, 2], // 685 GetPlayerXYZ
    &[2, 2], // 686 ScrCmd_686
    &[2], // 687 ScrCmd_687
    &[2, 2], // 688 GetPartySlotWithFatefulEncounter
    &[2], // 689 CommSanitizeParty
    &[2, 2], // 690 DaycareSanitizeMon
    &[2], // 691 ScrCmd_691
    &[1, 1, 1, 1, 2, 2], // 692 BufferBattleHallStreak
    &[2], // 693 BattleHallCountUsedSpecies
    &[2], // 694 BattleHallGetTotalStreak
    &[2], // 695 ScrCmd_695
    &[2], // 696 ScrCmd_696
    &[2], // 697 ScrCmd_697
    &[1, 2, 2], // 698 FollowerPokeIsEventTrigger
    &[], // 699 ScrCmd_699
    &[], // 700 ScrCmd_700
    &[2, 2], // 701 MonHasItem
    &[], // 702 BattleTowerSetUpMultiBattle
    &[2], // 703 SetPlayerVolume
    &[2, 2], // 704 ScrCmd_704
    &[2, 4], // 705 ScrCmd_705
    &[2], // 706 ScrCmd_706
    &[2, 2], // 707 CheckMonSeen
    &[2], // 708 ScrCmd_708
    &[], // 709 ScrCmd_709
    &[], // 710 ScrCmd_710
    &[], // 711 FollowMonInteract
    &[1], // 712 ScrCmd_712
    &[1], // 713 AlphPuzzle
    &[1], // 714 OpenAlphHiddenRoom
    &[], // 715 UpdateDaycareMonObjects
    &[], // 716 ScrCmd_716
    &[2], // 717 ScrCmd_717
    &[1, 2], // 718 ScrCmd_718
    &[2, 2], // 719 ScrCmd_719
    &[2], // 720 ScrCmd_720
    &[2], // 721 ScrCmd_721
    &[1, 1, 2, 2, 2], // 722 ScrCmd_722
    &[1, 1, 2, 2, 2], // 723 ScrCmd_723
    &[2, 2], // 724 ScrCmd_724
    &[1, 2], // 725 ScrCmd_725
    &[], // 726 ProcessSoundplate
    &[2], // 727 GetFollowPokePartyIndex
    &[1, 1], // 728 ScrCmd_728
    &[2], // 729 ScrCmd_729
    &[2], // 730 ScrCmd_730
    &[], // 731 ScrCmd_731
    &[1], // 732 ScrCmd_732
    &[1, 2], // 733 ScrCmd_733
    &[1], // 734 ScrCmd_734
    &[2], // 735 ScrCmd_735
    &[], // 736 ClearKurtApricorn
    &[2], // 737 ScrCmd_737
    &[2], // 738 GetTotalApricornCount
    &[], // 739 ScrCmd_739
    &[2, 2], // 740 ScrCmd_740
    &[2, 2, 2, 2], // 741 ScrCmd_741
    &[2, 2, 2], // 742 ScrCmd_742
    &[2], // 743 ScrCmd_743
    &[], // 744 CreatePokeathlonFriendshipRoomStatues
    &[1, 2], // 745 BufferPokeathlonCourseName
    &[], // 746 TouchscreenMenuHide
    &[], // 747 TouchscreenMenuShow
    &[2], // 748 GetMenuChoice
    &[1, 1, 1, 1, 2], // 749 MenuInitStdGmm
    &[1, 1, 1, 1, 2], // 750 MenuInit
    &[2, 2, 2], // 751 MenuItemAdd
    &[], // 752 MenuExec
    &[2, 2, 2], // 753 RockSmashItemCheck
    &[2], // 754 TryHeadbuttEncounter
    &[], // 755 LegendCutsceneClearBellAnimBegin
    &[], // 756 LegendCutsceneClearBellAnimEnd
    &[], // 757 LegendCutsceneClearBellRiseFromBag
    &[2], // 758 LegendCutsceneClearBellShimmer
    &[], // 759 LegendCutsceneLugiaEyeGlimmerEffect
    &[], // 760 ScrCmd_760
    &[2], // 761 LegendCutsceneMoveCameraTo
    &[2], // 762 LegendCutscenePanCameraTo
    &[], // 763 LegendCutsceneWaitCameraPan
    &[], // 764 LegendCutsceneBirdFinalApproach
    &[], // 765 LegendCutsceneWavesOrLeavesEffectBegin
    &[], // 766 LegendCutsceneWavesOrLeavesEffectEnd
    &[], // 767 LegendCutsceneLugiaArrivesEffectBegin
    &[], // 768 LegendCutsceneLugiaArrivesEffectEnd
    &[], // 769 LegendCutsceneLugiaArrivesEffectCameraPan
    &[2], // 770 CheckSeenAllLetterUnown
    &[], // 771 ScrCmd_771
    &[], // 772 ScrCmd_772
    &[2], // 773 Cinematic
    &[2], // 774 ShowLegendaryWing
    &[2, 2], // 775 ScrCmd_775
    &[], // 776 GiveTogepiEgg
    &[2, 2], // 777 ScrCmd_777
    &[], // 778 GiveSpikyEarPichu
    &[2, 2], // 779 RadioMusicIsPlaying
    &[1, 1], // 780 CasinoGame
    &[2], // 781 KenyaCheckPartyOrMailbox
    &[], // 782 MartSell
    &[1], // 783 SetFollowMonInhibitState
    &[1, 1], // 784 ScriptOverlayCmd
    &[1, 2], // 785 BugContestAction
    &[1], // 786 BufferBugContestWinner
    &[2, 2, 2], // 787 JudgeBugContest
    &[1, 2], // 788 BufferBugContestMonNick
    &[1], // 789 BugContestGetTimeLeft
    &[2, 2], // 790 IsBugContestantRegistered
    &[1, 2], // 791 CheckSafariZoneChallengeCompleted
    &[], // 792 UpdateSafariZoneIGT
    &[2, 2], // 793 BankTransaction
    &[2, 4], // 794 CheckBankBalance
    &[2, 2], // 795 ScrCmd_795
    &[], // 796 ScrCmd_796
    &[], // 797 ScrCmd_797
    &[2], // 798 BufferRulesetName
    &[2], // 799 ScrCmd_799
    &[2], // 800 ScrCmd_800
    &[2], // 801 ScrCmd_801
    &[], // 802 ScrCmd_802
    &[2, 2], // 803 ScrCmd_803
    &[1], // 804 ScrCmd_804
    &[], // 805 ScrCmd_805
    &[], // 806 ScrCmd_806
    &[2, 2], // 807 SetTrainerHouseSprite
    &[2], // 808 ScrCmd_808
    &[2], // 809 ShowTrainerHouseIntroMessage
    &[], // 810 ScrCmd_810
    &[2, 2], // 811 ScrCmd_811
    &[], // 812 ScrCmd_812
    &[2], // 813 MomGiftCheck
    &[], // 814 ScrCmd_814
    &[2], // 815 ScrCmd_815
    &[], // 816 UnownCircle
    &[1], // 817 ScrCmd_817
    &[], // 818 MystriStageGymmickInit
    &[], // 819 ScrCmd_819
    &[1], // 820 ScrCmd_820
    &[2, 2], // 821 GetBuenasPassword
    &[], // 822 ScrCmd_822
    &[2], // 823 ScrCmd_823
    &[2], // 824 ScrCmd_824
    &[2, 2], // 825 GetShinyLeafCount
    &[2], // 826 TryGiveShinyLeafCrown
    &[2, 2], // 827 GetPartyMonForm2
    &[2, 1, 2], // 828 MonAddContestValue
    &[2], // 829 ScrCmd_829
    &[2], // 830 ScrCmd_830
    &[2], // 831 ScrCmd_831
    &[2], // 832 ScrCmd_832
    &[2], // 833 ScrCmd_833
    &[2], // 834 ScrCmd_834
    &[2], // 835 ScrCmd_835
    &[2], // 836 CheckKyogreGroudonInParty
    &[2], // 837 ScrCmd_837
    &[2, 2], // 838 BankOrWalletIsFull
    &[2], // 839 SysSetSleepFlag
    &[2, 2], // 840 ScrCmd_840
    &[1], // 841 ScrCmd_841
    &[1], // 842 ScrCmd_842
    &[1, 2], // 843 BufferItemNameIndef
    &[1, 2], // 844 BufferItemNamePlural
    &[1, 2], // 845 BufferPartyMonSpeciesNameIndef
    &[1, 2, 2, 1], // 846 BufferSpeciesNameIndef
    &[1], // 847 BufferDPPtFriendStarterSpeciesNameIndef
    &[1, 2], // 848 BufferFashionNameIndef
    &[1, 2], // 849 BufferTrainerClassNameIndef
    &[1, 2], // 850 BufferSealNamePlural
    &[1], // 851 Capitalize
    &[1, 1], // 852 BufferDeptStoreFloorNo
];

impl Opcode {
    /// The command for opcode `op`, or `None` at or beyond
    /// [`OPCODE_COUNT`] (where the original asserts and stops).
    #[must_use]
    pub fn from_u16(op: u16) -> Option<Self> {
        ALL.get(usize::from(op)).copied()
    }

    /// The opcode number (`gScriptCmdTable` index).
    #[must_use]
    pub fn code(self) -> u16 {
        self as u16
    }

    /// pret's identifier for the handler (`ScrCmd_*`).
    #[must_use]
    pub fn name(self) -> &'static str {
        NAMES[self as usize]
    }

    /// The fixed operand sizes (bytes) the command's macro assembles,
    /// in read order. For the variable-layout commands this is the
    /// leading operand only.
    #[must_use]
    pub fn operands(self) -> &'static [u8] {
        OPERANDS[self as usize]
    }

    /// Whether the command's operand count depends on its first
    /// operand (`StrengthFlagAction`/`FlashAction`/`DefogAction`,
    /// `ScrCmd_465`, `MysteryGift`).
    #[must_use]
    pub fn is_variable_layout(self) -> bool {
        matches!(self as u16, 400..=402 | 465 | 489)
    }

    /// The index of the operand that is a relative code (or, for
    /// `ApplyMovement`, movement-list) target — a `.word \dest-.-4`,
    /// resolved as *address after the word* + value.
    #[must_use]
    pub fn relative_target(self) -> Option<usize> {
        match self as u16 {
            22 => Some(0),
            23 => Some(1),
            24 => Some(1),
            25 => Some(1),
            26 => Some(0),
            28 => Some(1),
            29 => Some(1),
            94 => Some(1),
            225 => Some(0),
            _ => None,
        }
    }

    /// Whether the target of [`Self::relative_target`] is a movement
    /// list rather than code.
    #[must_use]
    pub fn targets_movement(self) -> bool {
        self == Opcode::ApplyMovement
    }

    /// Whether a linear walk cannot continue past the command: `End`
    /// stops the context, `Return` pops, `GoTo` always branches.
    #[must_use]
    pub fn ends_block(self) -> bool {
        matches!(self, Opcode::End | Opcode::Return | Opcode::GoTo)
    }
}

/// One decoded instruction: where it sits, what it is, and its operand
/// values in macro order (bytes and halfwords widened; relative targets
/// left as the raw word — see [`Instruction::target`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    /// Offset of the opcode halfword within the bank member.
    pub offset: usize,
    /// The command.
    pub opcode: Opcode,
    /// Operand values, in read order.
    pub operands: Vec<u32>,
    /// Total encoded length in bytes (opcode included).
    pub len: usize,
}

impl Instruction {
    /// The offset just past this instruction — where a fall-through
    /// continues, and the base every relative operand is measured from
    /// (`ctx->script_ptr` after the word has been read).
    #[must_use]
    pub fn next(&self) -> usize {
        self.offset + self.len
    }

    /// The absolute target of the relative-branch (or movement-list)
    /// operand, if the command has one: the word is added to the
    /// address after it, with pointer-arithmetic wraparound (a
    /// backward branch is a two's-complement word).
    #[must_use]
    pub fn target(&self) -> Option<usize> {
        let index = self.opcode.relative_target()?;
        let value = *self.operands.get(index)?;
        // The relative word is the last operand of every such command,
        // so "after the word" is the instruction end.
        Some(self.next().wrapping_add(value as i32 as isize as usize))
    }
}

/// Reads the little-endian operand of `size` bytes at `at`.
fn read_operand(code: &[u8], at: usize, size: u8) -> Option<u32> {
    let end = at.checked_add(usize::from(size))?;
    let bytes = code.get(at..end)?;
    Some(bytes
        .iter()
        .rev()
        .fold(0u32, |acc, &b| (acc << 8) | u32::from(b)))
}

/// Decodes the instruction at `offset` of `code`.
///
/// # Errors
/// [`ScriptError::Truncated`] when the opcode or an operand runs past
/// the end of `code`; [`ScriptError::UnknownOpcode`] when the opcode is
/// at or beyond [`OPCODE_COUNT`] (the original asserts and stops).
pub fn decode_at(code: &[u8], offset: usize) -> Result<Instruction, ScriptError> {
    let raw = read_operand(code, offset, 2).ok_or(ScriptError::Truncated { offset })?;
    let raw = raw as u16;
    let opcode = Opcode::from_u16(raw).ok_or(ScriptError::UnknownOpcode {
        opcode: raw,
        offset,
    })?;
    let mut at = offset + 2;
    let mut operands = Vec::with_capacity(opcode.operands().len());
    let read = |at: &mut usize, size: u8| -> Result<u32, ScriptError> {
        let value = read_operand(code, *at, size).ok_or(ScriptError::Truncated { offset })?;
        *at += usize::from(size);
        Ok(value)
    };
    for &size in opcode.operands() {
        operands.push(read(&mut at, size)?);
    }
    if opcode.is_variable_layout() {
        // The macro's `.if` arms, `asm/macros/script.inc`:
        let first = operands[0];
        match opcode.code() {
            // `.byte action` then `.short var` when action == 2.
            400..=402 => {
                if first == 2 {
                    operands.push(read(&mut at, 2)?);
                }
            }
            // `.short a0`; a0 <= 3: two halfwords; a0 != 6: one.
            465 => {
                if first <= 3 {
                    operands.push(read(&mut at, 2)?);
                    operands.push(read(&mut at, 2)?);
                } else if first != 6 {
                    operands.push(read(&mut at, 2)?);
                }
            }
            // `.short a0`; 1..=3: one halfword; 5 or 6: two.
            489 => {
                if (1..=3).contains(&first) {
                    operands.push(read(&mut at, 2)?);
                } else if first == 5 || first == 6 {
                    operands.push(read(&mut at, 2)?);
                    operands.push(read(&mut at, 2)?);
                }
            }
            _ => {}
        }
    }
    Ok(Instruction {
        offset,
        opcode,
        operands,
        len: at - offset,
    })
}

/// Reads the movement list at `offset`: `(command, length)` halfword
/// pairs up to and including the [`MOVEMENT_STEP_END`] terminator.
///
/// # Errors
/// [`ScriptError::Truncated`] when the list runs off the end of `code`
/// before terminating, or exceeds 1024 steps (no retail list comes
/// near; a runaway walk means the offset was not a list).
pub fn decode_movement(code: &[u8], offset: usize) -> Result<Vec<MovementCommand>, ScriptError> {
    let mut steps = Vec::new();
    let mut at = offset;
    loop {
        let command = read_operand(code, at, 2).ok_or(ScriptError::Truncated { offset })? as u16;
        let length = read_operand(code, at + 2, 2).ok_or(ScriptError::Truncated { offset })? as u16;
        at += 4;
        steps.push(MovementCommand { command, length });
        if command == MOVEMENT_STEP_END {
            return Ok(steps);
        }
        if steps.len() > 1024 {
            return Err(ScriptError::Truncated { offset });
        }
    }
}

/// A whole bank, decoded by recursive descent from its entry points.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disassembly {
    /// Every reachable instruction, by offset.
    pub instructions: BTreeMap<usize, Instruction>,
    /// Every reachable movement list, by offset.
    pub movements: BTreeMap<usize, Vec<MovementCommand>>,
    /// How many reachable instructions carry each opcode.
    pub histogram: Vec<u32>,
}

impl Disassembly {
    /// The number of reachable instructions.
    #[must_use]
    pub fn instruction_count(&self) -> usize {
        self.instructions.len()
    }

    /// The opcodes that occur at least once, ascending.
    #[must_use]
    pub fn opcodes_used(&self) -> Vec<Opcode> {
        self.histogram
            .iter()
            .enumerate()
            .filter(|&(_, &n)| n != 0)
            .filter_map(|(i, _)| Opcode::from_u16(i as u16))
            .collect()
    }

    /// `(opcode, count)` for every opcode that occurs, ascending by opcode.
    #[must_use]
    pub fn counts(&self) -> Vec<(Opcode, u32)> {
        self.opcodes_used()
            .into_iter()
            .map(|op| (op, self.histogram[op as usize]))
            .collect()
    }
}

/// Walks every script of `bank` from its entry point: linear until a
/// block-ending command, following every relative branch (both arms of
/// a conditional) and recording every movement list. Instructions are
/// counted once each, however many paths reach them.
///
/// # Errors
/// The first decode error met on any path — an unknown opcode or an
/// operand past the member end. Retail banks decode clean; an error
/// here means the operand table and the ROM disagree.
pub fn disassemble(bank: &ScriptBank) -> Result<Disassembly, ScriptError> {
    let entries: Vec<usize> = (0..bank.script_count()).collect();
    disassemble_entries(bank, &entries)
}

/// [`disassemble`] restricted to the scripts at entry-table indices
/// `entries` (and everything they branch to) — how the inventory
/// tests walk only the `std_misc` scripts a map bank `CallStd`s.
/// Indices past the table are skipped.
///
/// # Errors
/// As [`disassemble`].
pub fn disassemble_entries(bank: &ScriptBank, entries: &[usize]) -> Result<Disassembly, ScriptError> {
    let code = bank.bytes();
    let mut out = Disassembly {
        histogram: vec![0; OPCODE_COUNT],
        ..Disassembly::default()
    };
    let mut work: Vec<usize> = entries
        .iter()
        .filter_map(|&i| bank.script_offset(i))
        .collect();
    while let Some(mut at) = work.pop() {
        loop {
            if out.instructions.contains_key(&at) {
                break;
            }
            let insn = decode_at(code, at)?;
            let next = insn.next();
            let ends = insn.opcode.ends_block();
            if let Some(target) = insn.target() {
                if insn.opcode.targets_movement() {
                    if let std::collections::btree_map::Entry::Vacant(slot) =
                        out.movements.entry(target)
                    {
                        slot.insert(decode_movement(code, target)?);
                    }
                } else {
                    work.push(target);
                }
            }
            out.histogram[insn.opcode as usize] += 1;
            out.instructions.insert(at, insn);
            if ends {
                break;
            }
            at = next;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_the_full_853_and_round_trips() {
        assert_eq!(ALL.len(), OPCODE_COUNT);
        for (i, op) in ALL.iter().enumerate() {
            assert_eq!(*op as usize, i);
            assert_eq!(Opcode::from_u16(i as u16), Some(*op));
            assert_eq!(op.name(), NAMES[i]);
        }
        assert_eq!(Opcode::from_u16(853), None);
        assert_eq!(Opcode::from_u16(0xFD13), None, "SCRDEF_END is not a command");
        // pret's spellings, kept verbatim.
        assert_eq!(Opcode::Nop.name(), "ScrCmd_Nop");
        assert_eq!(Opcode::End.code(), 2);
        assert_eq!(Opcode::UnsetPhoneCallTrigger.code(), 149);
        assert_eq!(Opcode::UnsetPhoneCallTrigger.name(), "UnsetPhoneCallTrigger");
        assert_eq!(Opcode::Dummy486.name(), "ScrCmd_Dummy");
        assert_eq!(Opcode::BufferDeptStoreFloorNo.code(), 852);
    }

    #[test]
    fn operand_layouts_match_the_macros() {
        // Wait frames, var — two halfwords.
        assert_eq!(Opcode::Wait.operands(), &[2, 2]);
        // LoadByte reg, val — two bytes; LoadWord reg, val — byte + word.
        assert_eq!(Opcode::LoadByte.operands(), &[1, 1]);
        assert_eq!(Opcode::LoadWord.operands(), &[1, 4]);
        // GoToIf condition, dest — byte + relative word.
        assert_eq!(Opcode::GoToIf.operands(), &[1, 4]);
        assert_eq!(Opcode::GoToIf.relative_target(), Some(1));
        assert_eq!(Opcode::GoTo.relative_target(), Some(0));
        assert_eq!(Opcode::ApplyMovement.operands(), &[2, 4]);
        assert!(Opcode::ApplyMovement.targets_movement());
        assert_eq!(Opcode::NPCMsg.operands(), &[1]);
        assert_eq!(Opcode::FadeScreen.operands(), &[2, 2, 2, 2]);
        assert_eq!(Opcode::Warp.operands(), &[2, 2, 2, 2, 2]);
        assert!(Opcode::End.operands().is_empty());
        assert!(Opcode::MysteryGift.is_variable_layout());
        assert!(!Opcode::SetFlag.is_variable_layout());
    }

    #[test]
    fn decodes_fixed_and_variable_layouts() {
        // SetFlag 0x0123.
        let code = [30, 0, 0x23, 0x01];
        let insn = decode_at(&code, 0).unwrap();
        assert_eq!(insn.opcode, Opcode::SetFlag);
        assert_eq!(insn.operands, vec![0x123]);
        assert_eq!(insn.len, 4);
        assert_eq!(insn.target(), None);

        // GoTo -8 (a backward branch to offset 0 from offset 8).
        let code = [0u8, 0, 22, 0, 0xF8, 0xFF, 0xFF, 0xFF];
        let insn = decode_at(&code, 2).unwrap();
        assert_eq!(insn.next(), 8);
        assert_eq!(insn.target(), Some(0));

        // FlashAction 2, var — the conditional halfword is read.
        let code = [0x91, 0x01, 2, 0x0C, 0x80];
        let insn = decode_at(&code, 0).unwrap();
        assert_eq!(insn.opcode, Opcode::FlashAction);
        assert_eq!(insn.operands, vec![2, 0x800C]);
        assert_eq!(insn.len, 5);
        // FlashAction 1 — no trailing halfword.
        let code = [0x91, 0x01, 1];
        assert_eq!(decode_at(&code, 0).unwrap().len, 3);

        // MysteryGift 5 takes two halfwords, 4 takes none.
        let code = [0xE9, 0x01, 5, 0, 1, 0, 2, 0];
        assert_eq!(decode_at(&code, 0).unwrap().operands, vec![5, 1, 2]);
        let code = [0xE9, 0x01, 4, 0];
        assert_eq!(decode_at(&code, 0).unwrap().len, 4);
        // ScrCmd_465: a0 <= 3 → two halfwords; a0 == 6 → none; else one.
        let code = [0xD1, 0x01, 6, 0];
        assert_eq!(decode_at(&code, 0).unwrap().len, 4);
        let code = [0xD1, 0x01, 7, 0, 9, 0];
        assert_eq!(decode_at(&code, 0).unwrap().operands, vec![7, 9]);

        // Truncated operand and unknown opcode.
        assert!(matches!(
            decode_at(&[30, 0, 0x23], 0),
            Err(ScriptError::Truncated { offset: 0 })
        ));
        assert!(matches!(
            decode_at(&[0x55, 0x03], 0),
            Err(ScriptError::UnknownOpcode {
                opcode: 853,
                offset: 0
            })
        ));
    }

    #[test]
    fn movement_lists_end_at_step_end() {
        let code = [12, 0, 3, 0, 254, 0, 0, 0];
        let list = decode_movement(&code, 0).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], MovementCommand { command: 12, length: 3 });
        assert_eq!(list[1].command, MOVEMENT_STEP_END);
        assert!(decode_movement(&code[..6], 0).is_err());
    }
}

