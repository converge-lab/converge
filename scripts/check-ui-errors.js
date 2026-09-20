// Browser regression checks against a local API build with synthetic responses only.
// Run this page function with Playwright; see docs/tasks/ui-error-handling.md.
async (page) => {
  const context = await page.context().browser().newContext({viewport:{width:1280,height:900}});
  page = await context.newPage();
  page.setDefaultTimeout(5000);
  const origin = 'http://127.0.0.1:8086';
  const uid = '01ARZ3NDEKTSV4RRFFQ69G5FAV';
  const gid = '01ARZ3NDEKTSV4RRFFQ69G5FAW';
  const pid = '01ARZ3NDEKTSV4RRFFQ69G5FAX';
  const did = '01ARZ3NDEKTSV4RRFFQ69G5FAY';
  const teammate = '01ARZ3NDEKTSV4RRFFQ69G5FAZ';
  const now = '2026-09-20T09:00:00Z';
  const me = {id:uid,provider:'local',subject:'ui-test',handle:'owner',name:'Test Owner'};
  const member = (user_id, handle, name, owner) => ({user_id,handle,name,owner,invited_by:uid,since:now});
  const roster = [member(uid,'owner','Test Owner',true)];
  const decision = {id:did,project_id:pid,status:'accepted',title:'Fixture decision',summary:'Fixture summary',context:null,consequences:null,alternatives:[],authors:[{user:uid}],evidence:[],code_evidence:[],captured_at:now};
  const plans = [];
  const scheduled = [];
  const requests = [];
  const tokens = [];
  let signedOut = false;
  const plan = (method, suffix, response, wait) => scheduled.push({method,suffix,response,wait});
  const json = (status, data) => ({status,contentType:'application/json',body:JSON.stringify(data)});
  const delayed = (method, suffix, response) => {
    let release;
    const wait = new Promise(resolve => { release=resolve; });
    plan(method,suffix,response,wait);
    return release;
  };
  const go = async hash => { await page.evaluate(hash => { location.hash=hash; },hash); };
  const alert = text => page.getByRole('alert').filter({hasText:text});
  const hasAlert = async text => { await alert(text).waitFor(); };
  const button = name => page.getByRole('button',{name,exact:true});
  let memberFailure = false;
  let searchFailure = true;
  let sourceFailure = false;
  let bootFailure = false;
  let searchGate = null;
  let oldSearchDone = null;
  const adds = [];
  const checks = [];
  const panics = [];
  const diagnostics = [];
  await page.exposeFunction("recordUiConsole", (method,args) => diagnostics.push({method,args}));
  await page.addInitScript(() => {
    for (const method of ["log","info","debug","warn","error","trace","dir","table"]) {
      console[method] = (...args) => { void window.recordUiConsole(method, args.map(String)); };
    }
  });
  const onError = e => panics.push(e.message);
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const envelope = (status, code, message) => ({status,contentType:'application/json',body:JSON.stringify({error:{code,message}})});
  const failure = envelope(400,'invalid','no user with handle `private-handle` — they need to sign in once first');
  const unavailable = envelope(503,'unavailable','storage unavailable');
  const internal = envelope(500,'internal','internal error');
  const pageOf = items => ({items,next_cursor:null});
  const invite = () => page.getByRole('button',{name:/Invite$/});
  const input = () => page.getByRole('textbox',{name:'Converge handle'});
  const add = () => page.getByRole('button',{name:'Add',exact:true});
  const open = async (handle) => { await invite().click(); await input().fill(handle); };
  const group = async () => { await page.goto(origin+'/#/group/settings'); await invite().waitFor({timeout:5000}); };
  const inline = () => page.locator('#invite-error');
  const dismiss = async () => { const close=page.locator('.cv-toast__close'); while(await close.count()) await close.first().click(); };
  page.on('pageerror',onError);
  await page.unrouteAll({behavior:'ignoreErrors'});
  await page.route(origin+'/api/v1/**', async route => {
    const url = route.request().url();
    const path = url.split('?')[0];
    const method = route.request().method();
    requests.push({method,path});
    const index = scheduled.findIndex(item => item.method===method && path.endsWith(item.suffix));
    if(index>=0) {
      const item=scheduled.splice(index,1)[0];
      if(item.wait) await item.wait;
      return route.fulfill(item.response);
    }
    const reply = data => route.fulfill({status:200,contentType:'application/json',body:JSON.stringify(data)});
    if(path.endsWith('/members')) {
      if(method==='POST') {
        adds.push(route.request().postDataJSON());
        const plan=plans.shift() || {response:failure};
        if(plan.wait) await plan.wait;
        if(plan.abort) return route.abort('failed');
        if(plan.success) { roster.push(member(teammate,adds[adds.length-1].handle,'Mixed Case',false)); return route.fulfill({status:204}); }
        return route.fulfill(plan.response);
      }
      return memberFailure ? route.fulfill(unavailable) : reply(roster);
    }
    if(path.endsWith('/me')) return signedOut ? route.fulfill(envelope(401,'unauthorized','unauthorized')) : reply(me);
    if(path.endsWith('/auth')) return reply({oidc:'Example provider'});
    if(path.endsWith('/tokens')) return reply(pageOf(tokens));
    if(path.endsWith('/groups')) return bootFailure ? route.fulfill(internal) : reply(pageOf([{id:gid,name:'UI test group',description:null,kind:'shared',owner:uid,created_at:now}]));
    if(path.endsWith('/users')) return reply(pageOf([me]));
    if(path.endsWith('/projects')) return reply(pageOf([{id:pid,group_id:gid,name:'Fixture project',description:null,repository:null,created_at:now}]));
    if(path.endsWith('/edges')) return reply({supersedes:[],superseded_by:[],related_to:[],related_by:[]});
    if(path.endsWith('/sources')) return sourceFailure ? route.fulfill(internal) : reply([]);
    if(path.endsWith('/session') && method==='DELETE') return route.fulfill(unavailable);
    if(path.endsWith('/decisions')) {
      if(url.includes('q=')) {
        if(searchGate && !url.includes('status=')) {
          const gate=searchGate; searchGate=null; await gate;
          await route.fulfill(internal); if(oldSearchDone) oldSearchDone(); return;
        }
        return searchFailure ? route.fulfill(internal) : reply(pageOf([]));
      }
      return reply(pageOf([decision]));
    }
    return reply(pageOf([]));
  });
  try {
    await page.goto('about:blank');
    await group();
    await open('private-handle');
    await add().click();
    await inline().filter({hasText:'no user with handle'}).waitFor({timeout:5000});
    assert(await page.locator('.cv-modal').isVisible(),'Failure closed the modal');
    assert(await input().inputValue()==='private-handle','Failure lost the handle');
    const rect=await inline().boundingBox();
    assert(rect && rect.y>=0 && rect.y+rect.height<=await page.evaluate(()=>innerHeight),'Inline failure is outside viewport');
    checks.push('invalid handle: modal and input retained, explanation inside viewport');

    for(const [response,text,label] of [
      [envelope(401,'unauthorized','unauthorized'),'Sign in again','401'],
      [envelope(409,'conflict','membership changed; retry'),'membership changed','409'],
      [internal,'Something went wrong','500'],
    ]) {
      plans.push({response}); await add().click();
      await inline().filter({hasText:text}).waitFor({timeout:5000});
      checks.push(label+': actionable inline failure');
    }
    plans.push({abort:true}); await add().click();
    await inline().filter({hasText:'Check your connection'}).waitFor({timeout:5000});
    checks.push('network failure: inline recovery message');

    let release;
    const gate=new Promise(resolve=>{release=resolve;});
    plans.push({wait:gate,response:failure});
    const before=adds.length;
    const sent=page.waitForRequest(r=>r.method()==='POST'&&r.url().endsWith('/members'));
    await add().click(); await sent;
    await page.getByRole('status').filter({hasText:'Adding teammate'}).waitFor({timeout:5000});
    assert(await add().isDisabled() && await input().isDisabled(),'Pending request still allows editing/submission');
    await add().dispatchEvent('click');
    release(); await inline().filter({hasText:'no user with handle'}).waitFor({timeout:5000});
    assert(adds.length===before+1,'Duplicate submission reached API');
    checks.push('pending state blocks duplicate submissions and recovers');

    await input().fill('  @MixedCase  ');
    plans.push({success:true}); await add().click();
    await page.locator('.cv-modal').waitFor({state:'detached',timeout:5000});
    await page.locator('.cv-memberrow').filter({hasText:'@MixedCase'}).waitFor({timeout:5000});
    assert(adds[adds.length-1].handle==='MixedCase','Provider handle was lowercased');
    checks.push('retry succeeds, preserves case, closes modal and reloads roster');
    await dismiss();

    for(const navigate of [false,true]) {
      let finish; const wait=new Promise(resolve=>{finish=resolve;});
      plans.push({wait,response:failure});
      await open('private-handle');
      const sent=page.waitForRequest(r=>r.method()==='POST'&&r.url().endsWith('/members'));
      await add().click(); await sent;
      if(navigate) {
        await page.evaluate(()=>{location.hash='#/search';});
        await page.getByPlaceholder('Search decisions, rationale, tags…').waitFor({timeout:5000});
      } else await page.getByRole('button',{name:'Cancel',exact:true}).click();
      finish();
      await page.locator('.cv-toast').filter({hasText:"Couldn't add teammate"}).waitFor({timeout:5000});
      checks.push('late failure survives '+(navigate?'navigation':'modal dismissal'));
      await dismiss();
      if(navigate) await group();
    }

    memberFailure=true; await page.reload();
    await page.getByRole('alert').filter({hasText:"Couldn't load members"}).waitFor({timeout:5000});
    assert(await invite().count()===0,'Failed roster exposed owner actions');
    memberFailure=false; await page.getByRole('button',{name:'Retry',exact:true}).click();
    await invite().waitFor({timeout:5000});
    checks.push('member loading failure is visible and retry restores controls');

    await page.goto(origin+'/#/search');
    const search=page.getByPlaceholder('Search decisions, rationale, tags…');
    await search.fill('query');
    await page.getByRole('alert').filter({hasText:"Couldn't search decisions"}).waitFor({timeout:5000});
    assert(await page.getByText('No matches',{exact:true}).count()===0,'Search failure pretends to be an empty result');
    searchFailure=false; await page.getByRole('button',{name:'Retry',exact:true}).click();
    await page.getByText('No matches',{exact:true}).waitFor({timeout:5000});
    checks.push('failed search differs from successful empty search; retry works');

    let releaseOld; searchGate=new Promise(resolve=>{releaseOld=resolve;});
    const oldDone=new Promise(resolve=>{oldSearchDone=resolve;});
    const oldSent=page.waitForRequest(r=>r.url().includes('q=old'));
    await search.fill('old'); await oldSent;
    await page.getByRole('combobox').nth(1).selectOption('accepted');
    await page.getByText('No matches',{exact:true}).waitFor({timeout:5000});
    releaseOld(); await oldDone; await page.waitForTimeout(100);
    assert(await page.getByRole('alert').count()===0,'Old query overwrote a newer filter result');
    checks.push('stale search failure cannot overwrite newer filter results');

    sourceFailure=true; await page.goto(origin+'/#/decision/'+did);
    await page.getByRole('alert').filter({hasText:"Couldn't load decision sources"}).waitFor({timeout:5000});
    sourceFailure=false;
    await page.getByRole('button',{name:'Retry',exact:true}).click();
    await page.getByRole('alert').filter({hasText:"Couldn't load decision sources"}).waitFor({state:'detached',timeout:5000});
    checks.push('decision source failure is visible inline and retry recovers');
    await go('#/decision/01ARZ3NDEKTSV4RRFFQ69G5FB2');
    await hasAlert("This decision couldn't be found");
    checks.push('missing decision displays an explanation instead of an empty detail screen');

    await page.getByRole('button',{name:/Test Owner @owner/}).click();
    await page.locator('.cv-acctmenu__item--danger').click();
    await page.locator('.cv-toast').filter({hasText:"Couldn't sign out"}).waitFor({timeout:5000});
    checks.push('failed sign-out is visible');

    bootFailure=true; await page.reload();
    await page.getByText("Couldn't load decision memory",{exact:true}).waitFor({timeout:5000});
    bootFailure=false; await page.getByRole('button',{name:'Retry',exact:true}).click();
    await page.getByRole('navigation',{name:'Main navigation'}).waitFor({timeout:5000});
    checks.push('boot error retry restores application');

    // Create and delete dialogs keep their draft and original route on refusal.
    for (const kind of ['group','project']) {
      await group();
      await button('New '+kind).click();
      const draft=page.locator('.cv-modal input');
      await draft.fill('draft-'+kind);
      const release=delayed('POST','/'+kind+'s',internal);
      await button('Create '+kind).click();
      await page.getByRole('status').filter({hasText:'Working…'}).waitFor();
      assert(await button('Create '+kind).isDisabled(),'Create was not disabled');
      const before=requests.filter(r=>r.method==='POST'&&r.path.endsWith('/'+kind+'s')).length;
      await button('Create '+kind).dispatchEvent('click');
      release();
      await hasAlert("Couldn't create "+kind);
      assert(await draft.inputValue()==='draft-'+kind,'Create failure lost draft');
      assert(requests.filter(r=>r.method==='POST'&&r.path.endsWith('/'+kind+'s')).length===before,'Create sent duplicate request');
      await button('Cancel').click();
      checks.push(kind+' creation preserves draft and prevents duplicate requests');
    }

    // Both existing settings forms show errors next to Save and retain inputs.
    for (const [kind,id,hash] of [['group',gid,'#/group/settings'],['project',pid,'#/project/'+pid+'/settings']]) {
      await page.goto('about:blank');
      await page.goto(origin+'/'+hash);
      await page.locator('.cv-setform input').first().fill('renamed-'+kind);
      await page.locator('.cv-setform input').nth(1).fill('retained description');
      plan('PATCH','/'+kind+'s/'+id,envelope(409,'conflict','name is already used'));
      await button('Save changes').click();
      await hasAlert("Couldn't save "+kind);
      assert(await page.locator('.cv-setform input').first().inputValue()==='renamed-'+kind,'Save lost name');
      assert(await page.locator('.cv-setform input').nth(1).inputValue()==='retained description','Save lost description');
      checks.push(kind+' settings retain entered values after server rejection');

      await button('Delete').click();
      const expected=kind==='group'?'UI test group':'Fixture project';
      await page.locator('.cv-modal input').fill(expected);
      plan('DELETE','/'+kind+'s/'+id,envelope(409,'conflict','Evidence is still referenced.'));
      await button('Delete '+kind).click();
      await hasAlert("Couldn't delete "+kind);
      assert(page.url().endsWith(hash),'Failed deletion navigated away');
      assert(await page.locator('.cv-modal input').inputValue()===expected,'Deletion lost confirmation');
      checks.push(kind+' deletion refusal keeps its confirmation and original screen');
      plan('DELETE','/'+kind+'s/'+id,{status:204});
      await button('Delete '+kind).click();
      await page.locator('.cv-modal').waitFor({state:'detached'});
      await page.waitForFunction(() => location.hash==='#/');
      await page.locator('.cv-toast').filter({hasText:'deleted.'}).waitFor();
      checks.push(kind+' deletion retry closes the dialog and navigates only on success');
    }

    await page.goto('about:blank');
    await group();
    plan('DELETE','/members/'+teammate,internal);
    await button('Remove Mixed Case').click();
    await hasAlert("Couldn't remove teammate");
    assert(await page.locator('.cv-memberrow').filter({hasText:'@MixedCase'}).count()===1,'Removal failure hid the member');
    plan('DELETE','/members/'+teammate,{status:204});
    await button('Remove Mixed Case').click();
    await page.locator('.cv-memberrow').filter({hasText:'@MixedCase'}).waitFor({state:'detached'});
    checks.push('member removal failure keeps the row; retry updates the roster');
    await dismiss();

    // A failure after a generic dialog closes reaches the shell. A later
    // unrelated success cannot replace it, and a second failure can coexist.
    await button('New project').click();
    await page.locator('.cv-modal input').fill('late-project');
    const finishProject=delayed('POST','/projects',internal);
    await button('Create project').click();
    await page.getByRole('status').filter({hasText:'Working…'}).waitFor();
    await button('Cancel').click();
    await go('#/settings');
    await page.getByPlaceholder("What's this token for? (laptop, ci, …)").waitFor();
    finishProject();
    await page.locator('.cv-toast').filter({hasText:"Couldn't create project"}).waitFor();
    await button('New group').click();
    await page.locator('.cv-modal input').fill('late-group');
    const finishGroup=delayed('POST','/groups',internal);
    await button('Create group').click();
    await page.getByRole('status').filter({hasText:'Working…'}).waitFor();
    await button('Cancel').click();
    finishGroup();
    await page.locator('.cv-toast').filter({hasText:"Couldn't create group"}).waitFor();
    assert(await page.locator('.cv-toast[role="alert"]').count()===2,'Independent errors replaced each other');
    await button('New project').click();
    await page.locator('.cv-modal input').fill('new-project');
    plan('POST','/projects',json(201,{id:'01ARZ3NDEKTSV4RRFFQ69G5FB0'}));
    await button('Create project').click();
    await page.locator('.cv-modal').waitFor({state:'detached'});
    await page.locator('.cv-toast').filter({hasText:'Project created.'}).waitFor();
    assert(await page.locator('.cv-toast[role="alert"]').count()===2,'Success erased an unrelated error');
    await page.locator('.cv-toast--ok').waitFor({state:'detached'});
    assert(await page.locator('.cv-toast[role="alert"]').count()===2,'Success timer erased errors');
    checks.push('late dialog failures coexist and survive unrelated success and its timer');
    await dismiss();

    // Token load errors are not an empty state; create/revoke preserve their
    // forms, and a late mint never exposes its secret in a global notice.
    await page.goto('about:blank');
    plan('GET','/tokens',internal);
    await page.goto(origin+'/#/settings');
    await hasAlert("Couldn't load tokens");
    assert(await page.getByText('No active tokens.',{exact:false}).count()===0,'Failed token load looked empty');
    await button('Retry').click();
    await page.getByText('No active tokens.',{exact:false}).waitFor();
    checks.push('token list distinguishes failure from empty and supports retry');
    const tokenLabel=page.getByPlaceholder("What's this token for? (laptop, ci, …)");
    await tokenLabel.fill('my laptop');
    let finishToken=delayed('POST','/tokens',internal);
    await button('Create token').click();
    await page.getByRole('status').filter({hasText:'Creating token…'}).waitFor();
    assert(await button('Create token').isDisabled(),'Token creation allows duplicates');
    finishToken();
    await hasAlert("Couldn't create token");
    assert(await tokenLabel.inputValue()==='my laptop','Token creation failure lost label');
    checks.push('token creation shows pending and keeps label after failure');
    const tokenId='01ARZ3NDEKTSV4RRFFQ69G5FB1';
    tokens.push({id:tokenId,user_id:uid,label:'my laptop',created_at:now});
    plan('POST','/tokens',json(201,{id:tokenId,token:'cvg_synthetic_secret'}));
    await button('Create token').click();
    await page.getByText('cvg_synthetic_secret',{exact:true}).waitFor();
    await page.locator('.cv-tokens__revoke').click();
    plan('DELETE','/tokens/'+tokenId,internal);
    await button('Revoke').click();
    await hasAlert("Couldn't revoke token");
    assert(await page.locator('.cv-tokens__row').count()===1,'Failed revocation removed token');
    assert(await page.locator('.cv-tokens__confirm').count()===1,'Failed revocation lost confirmation');
    plan('DELETE','/tokens/'+tokenId,{status:204});
    await button('Revoke').click();
    await page.locator('.cv-tokens__row').waitFor({state:'detached'});
    tokens.length=0;
    checks.push('token create success reveals secret locally; revoke failure retains row and retry removes it');
    await dismiss();
    await tokenLabel.fill('late token');
    finishToken=delayed('POST','/tokens',internal);
    await button('Create token').click();
    await page.getByRole('status').filter({hasText:'Creating token…'}).waitFor();
    await go('#/search');
    await page.getByPlaceholder('Search decisions, rationale, tags…').waitFor();
    finishToken();
    await page.locator('.cv-toast').filter({hasText:"Couldn't create token"}).waitFor();
    await dismiss();
    await go('#/settings');
    await tokenLabel.fill('late token');
    finishToken=delayed('POST','/tokens',json(201,{id:tokenId,token:'cvg_late_secret'}));
    await button('Create token').click();
    await page.getByRole('status').filter({hasText:'Creating token…'}).waitFor();
    await go('#/search');
    await page.getByPlaceholder('Search decisions, rationale, tags…').waitFor();
    finishToken();
    await page.locator('.cv-toast').filter({hasText:'you left before its secret could be shown'}).waitFor();
    assert(!(await page.locator('body').innerText()).includes('cvg_late_secret'),'Late secret leaked into notification');
    checks.push('token outcomes after navigation stay visible without exposing secrets');
    await dismiss();

    // Device lookup and decision have separate visible, retryable outcomes.
    await go('#/pair');
    const code=page.getByPlaceholder('XXXX-XXXX');
    await code.fill('ABCD-EFGH');
    plan('GET','/device/ABCD-EFGH',{status:404});
    await button('Look up').click();
    await hasAlert('No pending request');
    plan('GET','/device/ABCD-EFGH',internal);
    await button('Look up').click();
    await hasAlert("Couldn't look up pairing code");
    assert(await code.inputValue()==='ABCD-EFGH','Lookup failure lost code');
    plan('GET','/device/ABCD-EFGH',json(200,{user_code:'ABCD-EFGH',client_name:'Fixture CLI',expires_at:now}));
    await button('Look up').click();
    await button('Approve').waitFor();
    let finishPair=delayed('POST','/device/ABCD-EFGH',internal);
    await button('Approve').click();
    await page.getByRole('status').filter({hasText:'Contacting Converge…'}).waitFor();
    assert(await button('Approve').isDisabled() && await button('Deny').isDisabled(),'Pair decision permits duplicates');
    finishPair();
    await hasAlert("Couldn't complete device pairing");
    assert(await button('Approve').isEnabled(),'Pair failure cannot be retried');
    checks.push('pairing distinguishes missing code, lookup failure and approval failure, and allows retry');
    finishPair=delayed('POST','/device/ABCD-EFGH',internal);
    await button('Deny').click();
    await page.getByRole('status').filter({hasText:'Contacting Converge…'}).waitFor();
    await go('#/search');
    await page.getByPlaceholder('Search decisions, rationale, tags…').waitFor();
    finishPair();
    await page.locator('.cv-toast').filter({hasText:"Couldn't complete device pairing"}).waitFor();
    checks.push('device decision failure survives navigation');
    await dismiss();

    // Login option loading can be retried independently from token sign-in.
    signedOut=true;
    plan('GET','/auth',internal);
    await page.reload();
    await hasAlert("Couldn't load sign-in options");
    await button('Retry sign-in options').click();
    await page.getByRole('link',{name:'Sign in with Example provider'}).waitFor();
    await page.getByPlaceholder('cvg_…').fill('cvg_bad_secret');
    plan('POST','/session',envelope(401,'unauthorized','unauthorized'));
    await button('Sign in').click();
    await hasAlert("That token isn't recognized.");
    assert(await page.getByPlaceholder('cvg_…').inputValue()==='cvg_bad_secret','Sign-in error cleared entered token');
    checks.push('provider discovery retries; rejected sign-in is visible and retains input');
    signedOut=false;

    // The reported invitation problem must be readable on a narrow screen.
    await page.setViewportSize({width:390,height:844});
    await page.goto('about:blank');
    await group();
    await open('private-handle');
    await add().click();
    await inline().filter({hasText:'no user with handle'}).waitFor();
    const mobileRect=await inline().boundingBox();
    assert(mobileRect && mobileRect.y>=0 && mobileRect.y+mobileRect.height<=844,'Mobile invitation error is offscreen');
    checks.push('invitation error is visible in a 390×844 viewport');

    assert(panics.length===0,'Browser exceptions: '+panics.join('; '));
    assert(diagnostics.length===0,'UI wrote to console: '+JSON.stringify(diagnostics));
    checks.push('no browser exceptions or application console calls');
    return {passed:checks.length,checks};
  } catch (error) {
    throw new Error(String(error)+'\nCompleted: '+checks.join('; ')+'\nPage: '+(await page.locator('body').innerText()).slice(0,2500)+'\nExceptions: '+JSON.stringify(panics)+'\nConsole calls: '+JSON.stringify(diagnostics));
  } finally {
    page.removeListener('pageerror',onError);
    await context.close();
  }
}
