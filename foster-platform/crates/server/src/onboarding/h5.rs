use axum::response::Html;

pub async fn login_page() -> Html<&'static str> {
    Html(LOGIN_HTML)
}

pub async fn service_page() -> Html<&'static str> {
    Html(SERVICE_HTML)
}

const LOGIN_HTML: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>阴阳师寄养 · 登录</title>
<style>
:root{color-scheme:dark;--bg:#151719;--panel:#1d2023;--line:#32363a;--text:#e6e8ea;--muted:#91979d;--accent:#7f9a8b;--warn:#b69a72}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC",sans-serif}
.wrap{max-width:680px;margin:0 auto;padding:28px 18px}.card{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:22px}
h1{font-size:20px;margin:0 0 6px}p{margin:6px 0;color:var(--muted)}.status{margin:20px 0;padding:12px;border:1px solid var(--line);border-radius:8px}
#qr{display:none;max-width:320px;width:100%;margin:16px auto;border-radius:8px}.actions{display:flex;gap:10px;margin-top:18px}
button{border:1px solid var(--line);background:#292d30;color:var(--text);padding:10px 14px;border-radius:7px;cursor:pointer}button:disabled{opacity:.45;cursor:not-allowed}
.primary{background:#334039;border-color:#53675d}.meta{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:12px;color:var(--muted);word-break:break-all}
label{display:block;margin-top:12px;color:var(--muted)}input,select{width:100%;margin-top:5px;padding:10px 11px;border:1px solid var(--line);border-radius:7px;background:#16191b;color:var(--text);font:inherit}form{margin-top:18px;padding-top:16px;border-top:1px solid var(--line)}
</style>
</head>
<body><main class="wrap"><section class="card">
<h1>游戏账号登录</h1>
<p>请使用手机扫码完成登录。系统只保存账号识别信息，不保存密码。</p>
<div class="status"><strong id="status">正在连接…</strong><p id="detail"></p></div>
<img id="qr" alt="登录二维码">
<canvas id="qrCanvas" width="240" height="240" hidden></canvas>
<div id="qrText" class="meta"></div>
<div id="platformBox" style="display:none;margin-top:18px;text-align:center">
<p>扫码成功后，请选择账号所在的区服平台：</p>
<div class="actions" style="justify-content:center"><button id="android">安卓区</button><button id="ios">iOS 区</button></div>
</div>
<form id="identityBox" style="display:none">
<strong>填写账号识别信息</strong>
<p>游戏内弹窗、公告和“跳过”按钮由 Agent 在模拟器中处理，这里只填写账号信息。</p>
<label for="server">区服（可搜索）</label>
<input id="server" name="server" list="serverOptions" autocomplete="off" placeholder="选择或输入区服">
<datalist id="serverOptions"></datalist>
<label for="character">角色名</label>
<input id="character" name="character" autocomplete="off" placeholder="填写游戏内角色名">
<label for="uid">游戏 UID</label>
<input id="uid" name="uid" inputmode="numeric" autocomplete="off" placeholder="填写游戏 UID">
<div class="actions"><button id="submitIdentity" class="primary" type="submit">提交，开始校验</button></div>
</form>
<div class="actions"><button id="confirm" class="primary" disabled>确认这是我的账号</button></div>
</section></main>
<script>
const publicToken=decodeURIComponent(location.pathname.split('/').filter(Boolean).pop()||'');
const params=new URLSearchParams(location.hash.slice(1));
const controlToken=params.get('control');
const statusEl=document.getElementById('status'),detailEl=document.getElementById('detail');
const qr=document.getElementById('qr'),qrText=document.getElementById('qrText'),confirmBtn=document.getElementById('confirm');
const platformBox=document.getElementById('platformBox'),androidBtn=document.getElementById('android'),iosBtn=document.getElementById('ios');
const identityBox=document.getElementById('identityBox'),serverInput=document.getElementById('server'),characterInput=document.getElementById('character'),uidInput=document.getElementById('uid'),submitIdentity=document.getElementById('submitIdentity');
const qrCanvas=document.getElementById('qrCanvas');
let lastQrPayload='';
let selectedPlatform='';

const serverOptions={android:[],ios:[]};
function renderServerOptions(platform){
  document.getElementById('serverOptions').innerHTML=(serverOptions[platform]||[]).map(name=>'<option value="'+name.replaceAll('"','&quot;')+'"></option>').join('');
}

function renderQr(payload){
  const source=new Image();
  source.onload=()=>{
    if(source.naturalWidth===1280&&source.naturalHeight===720){
      const ctx=qrCanvas.getContext('2d');
      qrCanvas.width=220;qrCanvas.height=220;
      ctx.clearRect(0,0,220,220);
      ctx.drawImage(source,545,250,190,190,0,0,220,220);
      qr.src=qrCanvas.toDataURL('image/png');
    }else{
      qr.src=payload;
    }
    qr.style.display='block';
  };
  source.src=payload;
}

function render(data){
  statusEl.textContent=data.status||'UNKNOWN';
  const who=[data.characterName,data.serverName].filter(Boolean).join(' · ');
  detailEl.textContent=who||'等待游戏返回账号信息';
  confirmBtn.disabled=!(controlToken&&data.status==='VERIFYING_ACCOUNT'&&data.identityVerified);
  platformBox.style.display=(controlToken&&(data.status==='QR_READY'||data.status==='WAITING_SCAN'))?'block':'none';
  identityBox.style.display=(controlToken&&(data.status==='DETECTING_LOGIN'||data.status==='VERIFYING_ACCOUNT')&&!data.identityVerified)?'block':'none';
  if(data.identityVerifyReason)detailEl.textContent=data.identityVerifyReason;

  if(data.status==='QR_READY'||data.status==='WAITING_SCAN'){
    const qrUrl='/public/login/'+encodeURIComponent(publicToken)+'/qr?v='+encodeURIComponent(data.qrExpiresAt||Date.now());
    if(qrUrl!==lastQrPayload){lastQrPayload=qrUrl;renderQr(qrUrl)}
    qrText.textContent='';
  }else{
    qr.style.display='none';qrText.textContent='';
  }

  if(data.status==='SUCCESS'){
    statusEl.textContent='登录完成';
    detailEl.textContent=who||'账号已绑定，可以关闭此页面';
    confirmBtn.disabled=true;
  }
}

async function selectPlatform(platform){
  if(!controlToken)return;
  selectedPlatform=platform;
  renderServerOptions(platform);
  androidBtn.disabled=true;iosBtn.disabled=true;
  detailEl.textContent='已提交平台选择，正在继续登录…';
  const r=await fetch('/public/login/'+encodeURIComponent(controlToken)+'/platform',{
    method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({platform})
  });
  if(!r.ok){detailEl.textContent='平台选择失败，请重新点击';androidBtn.disabled=false;iosBtn.disabled=false;}
}

identityBox.addEventListener('submit',async event=>{
  event.preventDefault();
  if(!serverInput.value.trim()||!characterInput.value.trim()||!uidInput.value.trim()){
    detailEl.textContent='请填写区服、角色名和游戏 UID';
    return;
  }
  submitIdentity.disabled=true;
  detailEl.textContent='已提交账号信息，等待 Agent 截图/OCR 校验…';
  try{
    const r=await fetch('/public/login/'+encodeURIComponent(controlToken)+'/identity',{
      method:'POST',headers:{'content-type':'application/json'},
      body:JSON.stringify({serverName:serverInput.value.trim(),characterName:characterInput.value.trim(),gameUid:uidInput.value.trim()})
    });
    if(!r.ok){detailEl.textContent=r.status===409?'当前登录状态不能提交，请等扫码后再试':'提交失败，请检查填写内容后重试';return}
    const result=await r.json();detailEl.textContent=result.message;
    if(result.verified){identityBox.style.display='none';confirmBtn.disabled=false;}
  }catch{detailEl.textContent='网络异常，提交失败，请重试'}
  finally{submitIdentity.disabled=false}
});

async function refresh(){
  try{
    const r=await fetch('/public/login/'+encodeURIComponent(publicToken)+'/meta',{cache:'no-store'});
    if(!r.ok){statusEl.textContent=r.status===410?'登录链接已过期':'无法读取登录状态';return;}
    render(await r.json());
  }catch(e){statusEl.textContent='网络连接异常';}
}

confirmBtn.addEventListener('click',async()=>{
  if(!controlToken)return;
  confirmBtn.disabled=true;
  const r=await fetch('/public/login/'+encodeURIComponent(controlToken)+'/confirm',{
    method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({confirmed:true})
  });
  if(!r.ok){detailEl.textContent='确认失败，请稍后重试';}
  await refresh();
});
androidBtn.addEventListener('click',()=>selectPlatform('android'));
iosBtn.addEventListener('click',()=>selectPlatform('ios'));
refresh();setInterval(refresh,2000);
</script></body></html>"##;

#[cfg(test)]
mod tests {
    use super::LOGIN_HTML;

    #[test]
    fn login_page_keeps_game_popups_out_of_frontend() {
        assert!(LOGIN_HTML.contains("identityBox"));
        assert!(LOGIN_HTML.contains("serverOptions"));
        assert!(LOGIN_HTML.contains("Agent 在模拟器中处理"));
        assert!(!LOGIN_HTML.contains("游戏公告"));
        assert!(!LOGIN_HTML.contains("隐私协议"));
    }
}

const SERVICE_HTML: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>阴阳师寄养 · 服务状态</title>
<style>
:root{color-scheme:dark;--bg:#151719;--panel:#1d2023;--panel2:#23272a;--line:#34393e;--text:#e7e9ea;--muted:#92989e;--accent:#7e978a}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI","PingFang SC",sans-serif}
.wrap{max-width:860px;margin:auto;padding:24px 16px 60px}.head{display:flex;justify-content:space-between;align-items:flex-end;margin-bottom:16px}
h1{font-size:21px;margin:0}.muted{color:var(--muted)}.grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:12px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:18px}.wide{grid-column:1/-1}
.value{font-size:19px;margin-top:4px}.actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}
button{background:#292d30;border:1px solid var(--line);color:var(--text);padding:9px 12px;border-radius:7px;cursor:pointer}button:disabled{opacity:.4;cursor:not-allowed}
textarea{width:100%;min-height:130px;background:#16191b;color:var(--text);border:1px solid var(--line);border-radius:7px;padding:10px;font:12px/1.45 ui-monospace,SFMono-Regular,Menlo,monospace}
table{width:100%;border-collapse:collapse;margin-top:8px}td,th{text-align:left;padding:8px;border-bottom:1px solid #2d3135;font-size:12px}th{color:var(--muted);font-weight:500}
@media(max-width:620px){.grid{grid-template-columns:1fr}.wide{grid-column:auto}.head{align-items:flex-start;flex-direction:column;gap:4px}}
</style>
</head>
<body><main class="wrap">
<div class="head"><div><h1>我的寄养服务</h1><div id="account" class="muted">加载中…</div></div><div id="readonly" class="muted"></div></div>
<div class="grid">
<section class="card"><div class="muted">服务状态</div><div id="serviceStatus" class="value">—</div></section>
<section class="card"><div class="muted">今日寄养</div><div id="today" class="value">—</div></section>
<section class="card"><div class="muted">套餐</div><div id="plan" class="value">—</div></section>
<section class="card"><div class="muted">下次预计</div><div id="nextRun" class="value">—</div></section>
<section class="card wide"><div class="muted">临时暂停</div><div id="blockInfo">当前未暂停</div>
<div class="actions">
<button data-pause="1H">暂停 1 小时</button><button data-pause="2H">暂停 2 小时</button>
<button data-pause="4H">暂停 4 小时</button><button data-pause="TODAY">今天不上号</button>
<button id="resume">恢复运行</button></div></section>
<section class="card wide"><div class="muted">不上号时间段</div>
<p class="muted">JSON 数组，weekdayMask 1–127；支持跨午夜。</p>
<textarea id="quiet"></textarea><div class="actions"><button id="saveQuiet">保存时间段</button></div></section>
<section class="card wide"><div class="muted">最近执行</div><table><thead><tr><th>状态</th><th>计划时间</th><th>完成时间</th><th>结果</th></tr></thead><tbody id="jobs"></tbody></table></section>
</div></main>
<script>
const publicToken=decodeURIComponent(location.pathname.split('/').filter(Boolean).pop()||'');
const controlToken=new URLSearchParams(location.hash.slice(1)).get('control');
const $=id=>document.getElementById(id);
const fmt=v=>v?new Date(v).toLocaleString():'—';
let lastStatus=null;

function render(s){
  lastStatus=s;
  $('account').textContent=[s.characterName,s.serverName].filter(Boolean).join(' · ')||s.subscriptionNo;
  $('serviceStatus').textContent=s.reloginRequired?'需要重新登录':s.serviceStatus;
  $('today').textContent=s.todaySuccessCount+' / '+s.dailyTargetRuns;
  $('plan').textContent=s.planName+(s.resourceType?' · '+s.resourceType:'');
  $('nextRun').textContent=fmt(s.nextRunAt);
  $('blockInfo').textContent=s.effectiveBlockedUntil
    ?(s.blockReason+' 至 '+fmt(s.effectiveBlockedUntil)):'当前未暂停';
  if(document.activeElement!==$('quiet')){
    $('quiet').value=JSON.stringify(s.quietPeriods||[],null,2);
  }
  $('jobs').innerHTML=(s.recentJobs||[]).map(j=>
    '<tr><td>'+j.status+'</td><td>'+fmt(j.scheduledAt)+'</td><td>'+fmt(j.finishedAt)+'</td><td>'+(j.resultMessage||'')+'</td></tr>'
  ).join('');
}

async function refresh(){
  const r=await fetch('/r/'+encodeURIComponent(publicToken),{cache:'no-store'});
  if(r.ok)render(await r.json());
}
async function control(path,options){
  if(!controlToken)return;
  const r=await fetch('/r/'+encodeURIComponent(controlToken)+path,options);
  if(!r.ok)throw new Error('HTTP '+r.status);
  await refresh();
}

if(!controlToken){
  $('readonly').textContent='只读链接';
  document.querySelectorAll('button').forEach(b=>b.disabled=true);
}
document.querySelectorAll('[data-pause]').forEach(btn=>btn.addEventListener('click',()=>control('/pause',{
  method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({preset:btn.dataset.pause})
}).catch(()=>alert('暂停设置失败'))));
$('resume').addEventListener('click',()=>control('/pause',{method:'DELETE'}).catch(()=>alert('恢复失败')));
$('saveQuiet').addEventListener('click',async()=>{
  try{
    const raw=JSON.parse($('quiet').value||'[]');
    const quietPeriods=raw.map(x=>({
      weekdayMask:x.weekdayMask,startTime:x.startTime,endTime:x.endTime,
      beforeBufferMinutes:x.beforeBufferMinutes||0,afterBufferMinutes:x.afterBufferMinutes||0
    }));
    await control('/quiet-periods',{
      method:'PUT',headers:{'content-type':'application/json'},body:JSON.stringify({quietPeriods})
    });
  }catch(e){alert('时间段格式不正确');}
});
refresh();setInterval(refresh,5000);
</script></body></html>"##;
