
const callbacks = new Map(); let callbackId = 0;
const demoAccounts = [
 {login:'demo_main', displayName:'Main account', path:'C:/Demo/accounts/demo_main', lastLaunchAt:'1789000000', steamId:'76561198000000001',avatarPath:null,favorite:true,launchCount:12,hasAuthenticator:true,authenticatorImportedAt:'1788000000'},
 {login:'demo_second',displayName:'Second account',path:'C:/Demo/accounts/demo_second',lastLaunchAt:null,steamId:null,avatarPath:null,favorite:false,launchCount:0,hasAuthenticator:false,authenticatorImportedAt:null}
];
const demoSettings={version:1,workspace:'C:/Demo',mainSteamPathOverride:null,firstRunCompleted:true,language:'ru',defaultLaunchMode:'switch',sandboxieInstallAttempted:false,authMasterPasswordEnabled:false,authPollerEnabled:false,authPollerInterval:60,authAutoConfirmTrades:false,authAutoConfirmMarket:false};
const scenario=new URLSearchParams(location.search).get('scenario') || '';
if(scenario==='layout') {
 demoAccounts[0].avatarPath='fixture-avatar';
 demoAccounts.push({...demoAccounts[0],login:'demo_sandbox',displayName:'Sandbox account',favorite:false,launchCount:43});
 demoAccounts.push({...demoAccounts[0],login:'demo_long',displayName:'Очень длинное имя аккаунта для проверки',favorite:false,launchCount:108});
}
// Public documentation uses synthetic accounts, never a local Steam profile.
if(scenario==='showcase') {
 demoAccounts.splice(0,demoAccounts.length,...[
  ['demo_main','SHADOW',true,128,true,'2026-09-20T09:42:00+05:00'],
  ['demo_market','NIGHTSHIFT',true,64,true,'2026-09-20T09:18:00+05:00'],
  ['demo_sandbox','PIXEL',false,43,true,'2026-09-20T09:24:00+05:00'],
  ['demo_alt','GHOST',false,27,true,'2026-09-19T22:06:00+05:00'],
  ['demo_storage','VAULT',false,8,true,'2026-09-18T17:30:00+05:00'],
  ['demo_second','SECOND',false,0,false,null],
 ].map(([login,displayName,favorite,launchCount,hasAuthenticator,last],i)=>({
  login,displayName,favorite,launchCount,hasAuthenticator,
  path:`C:/Demo/accounts/${login}`,lastLaunchAt:last?String(Date.parse(last)/1000):null,
  steamId:hasAuthenticator?`7656119800000000${i+1}`:null,
  avatarPath:`fixture-avatar-${i}`,authenticatorImportedAt:hasAuthenticator?'1788000000':null,
 })));
}
const fixtureAvatar='data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#273b34"/><path d="M5 6h9v9H5zm13 0h9v9h-9zM9 19h15v7H9z" fill="#8bbfac"/></svg>')+'#';
const showcaseAvatars=['#7bdbac','#d4b87a','#a9b9eb','#bfa3d8','#95a7a1','#7597a0'].map((color,i)=>
 'data:image/svg+xml,'+encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32" shape-rendering="crispEdges"><rect width="32" height="32" fill="#18241f"/><path d="${[
  'M8 5h16v5h4v15H4V10h4z',
  'M12 3h8v6h6v6h4v8h-6v6H8v-6H2v-8h4V9h6z',
  'M6 6h7v7H6zm13 0h7v7h-7zM6 19h7v7H6zm13 0h7v7h-7zM13 13h6v6h-6z',
  'M10 4h12v4h4v20h-5v-5h-5v5h-5v-5H6V8h4z',
  'M6 5h20v22H6z',
  'M14 4h4v10h10v4H18v10h-4V18H4v-4h10z',
 ][i]}" fill="${color}"/><path d="M10 12h4v4h-4zm8 0h4v4h-4z" fill="#18241f"/></svg>`)+ '#');
let session=scenario==='session-error'?'needs_relogin':'ok';
let locked=scenario==='locked';
const brokenIcon='https://community.steamstatic.com/economy/image/fixture-missing-item';
const itemIcon=scenario==='broken-images'?brokenIcon:'https://community.steamstatic.com/economy/image/i0CoZ81Ui0m-9KwlBY1L_18myuGuq1wfhWSaZgMttyVfPaERSR0Wqmu7LAocGJKz2lu_XsnXwtmkJjSU91dh8bj35VTqVBP4io_frnIV7Kb5OaU-JqfHDzXFle0u4LY8Gy_kkRgisGzcm4v4J3vDOAQmDMdyRvlK7EcmeCU3yw';
let demoConfirmations=scenario==='empty'?[]:[
 {id:'101',nonce:'demo-sale',creator_id:'301',headline:'Selling for 136 RUB',summary:['Dreams & Nightmares Case'],type:3,type_name:'Market',accept:'Confirm',cancel:'Cancel',icon:itemIcon},
 {id:'102',nonce:'demo-trade',creator_id:'302',headline:'Trade with Demo partner',summary:['You give: 2 items','You receive: 1 item'],type:2,type_name:'Trade',accept:'Confirm',cancel:'Cancel',icon:''},
];
let finalizeCalls=0;
window.fixtureCalls=[];
window.__TAURI_INTERNALS__={metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},transformCallback:(fn)=>{let id=++callbackId;callbacks.set(id,fn);return id;},unregisterCallback:(id)=>callbacks.delete(id),convertFileSrc:(path)=>scenario==='showcase'?(showcaseAvatars[Number(path.slice(-1))]||fixtureAvatar):fixtureAvatar,invoke:async(cmd,args={})=>{
 window.fixtureCalls.push({cmd,args});
 switch(cmd){
 case 'get_settings':return {...demoSettings};
 case 'detect_main_steam':return {installDir:'C:/Steam',steamExe:'C:/Steam/steam.exe',steamappsDir:'C:/Steam/steamapps',autologinUser:'demo_main'};
 case 'detect_sandboxie':return {installed:true,installDir:'C:/Sandboxie',startExe:'C:/Sandboxie/Start.exe',version:'1.17'};
 case 'cleanup_stale_junctions':return {repaired:[],removed:[],errors:[]};
 case 'list_accounts':return demoAccounts;
 case 'verify_account':return {junction:{kind:'healthy'},configDirExists:true,hasLoginusersVdf:true,ready:true};
 case 'auth_status':return demoAccounts.map(a=>({login:a.login,hasAuthenticator:a.hasAuthenticator,hasSavedPassword:a.hasAuthenticator,autoLogin:{state:a.hasAuthenticator?'ready':'none',retry_at:null},enrollment:a.hasAuthenticator?'none':'pending',steamId:a.steamId,identityMismatch:false,accountName:a.login,importedAt:a.authenticatorImportedAt,sessionState:a.hasAuthenticator?session:'no_session'}));
 case 'auth_lock_status':return {enabled:scenario==='locked',unlocked:!locked,hasEncryptedFiles:scenario==='locked'};
 case 'auth_unlock':locked=false;return null;
 case 'auth_generate_code':return {code:scenario==='showcase'?({demo_main:'92M5P',demo_market:'7XK3D',demo_sandbox:'W8N4T',demo_alt:'3HJR6',demo_storage:'F2Q9C'}[args.login]||'ABCDE'):'ABCDE',generatedAt:Math.floor(Date.now()/1000),periodRemaining:30-(Math.floor(Date.now()/1000)%30)};
 case 'auth_session_state':return session;
 case 'auth_confirmations_list':if(locked)throw 'AUTH_LOCKED';if(session==='needs_relogin')throw 'Not ready: AUTH_AUTO_LOGIN_NEEDS_INPUT';return args.login==='demo_main'?demoConfirmations:[];
 case 'auth_confirmation_details':if(scenario==='details-error')throw 'CONF_DETAILS_UNAVAILABLE';return {partnerSteamId:'76561198000000002',giving:[{assetId:'11',appId:730,name:'Dreams & Nightmares Case',amount:'2',icon:itemIcon}],receiving:[{assetId:'12',appId:730,name:scenario==='showcase'?'Dreams & Nightmares Case':'Demo item with a long name to check wrapping',amount:'1',icon:scenario==='showcase'?itemIcon:brokenIcon}]};
 case 'auth_confirmations_respond':demoConfirmations=demoConfirmations.filter(c=>!args.ids.includes(c.id));return args.ids.map(id=>({id,success:true,message:''}));
 case 'auth_login_begin':return {clientId:'demo-client',requestId:'demo-request',steamId:'76561198000000001',weakToken:'',allowedConfirmations:[{confirmation_type:3,associated_message:''}],interval:2,guardSubmitted:true,autoGuardFailed:false};
 case 'auth_login_poll':session='ok';return {state:'Done',accessToken:'test-token',refreshToken:'test-token',accountName:args.login,steamId:'76561198000000001'};
 case 'auth_add_resume':return {phase:'finalize',phone_hint:'***1234',revocation_code:null};
 case 'cleanup_backups':return {removed:8,migrated:4,retained:10};
 case 'auth_login_refresh':session='ok';return null;
 case 'auth_poller_poke':return null;
 case 'auth_add_diagnose':return {guard:'email',already_has_mobile:false,phone_attached:false,phone_hint:'',suggested_path:'no-phone-fast'};
 case 'auth_add_create':return {phone_number_hint:'***1234',server_time:String(Math.floor(Date.now()/1000))};
 case 'auth_add_finalize':finalizeCalls++;return finalizeCalls < 3 ? {success:false,want_more:true,status:1,revocation_code:null} : {success:true,want_more:false,status:1,revocation_code:'R12345DEMO'};
 case 'set_default_launch_mode':demoSettings.defaultLaunchMode=args.mode;return null;
 case 'launch_shadow':case 'launch_account':return {kind:'switch',pid:1,previousAutologin:null};
 case 'set_account_favorite':{const account=demoAccounts.find(a=>a.login===args.login);if(account)account.favorite=args.value;return null;}
 case 'check_update':return {has_update:false,current:'0.2.2',latest:'0.2.2',notes:'',release_url:'',release_title:''};
 case 'discover_steam_accounts':return [{accountName:'import_one',personaName:'Import One',mostRecent:false},{accountName:'import_two',personaName:'Import Two',mostRecent:false}];
 case 'list_running_sandboxes':return ['layout','showcase'].includes(scenario)?[{login:'demo_sandbox',boxName:'SteamShadow_demo',pids:[1],startedAt:Math.floor(Date.now()/1000)-1080}]:[];
 case 'list_running_games':return [];
 case 'list_account_games':return [{appid:10,name:'Demo Game',installdir:'Demo',libraryPath:'C:/Steam/steamapps',iconPath:null}];
 case 'plugin:event|listen':return ++callbackId;
 case 'plugin:event|unlisten':case 'auth_poller_configure':case 'auth_login_cancel':case 'auth_add_cancel':case 'auth_add_persist':case 'auth_password_forget':case 'close_window':case 'minimize_window':return null;
 case 'save_settings':Object.assign(demoSettings,args.settings);return null;
 default:throw 'Review fixture: unsupported command '+cmd;
 }
}};
